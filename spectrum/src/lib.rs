//! The 48K ZX Spectrum machine, built over the pure `z80` core.
//!
//! `Spectrum` owns the CPU and a [`Board`] (memory + ULA + keyboard). The CPU
//! borrows `&mut Board` for each `step()`; the `Board` is what implements
//! [`z80::Bus`], so the borrow checker stays happy and all timing lives in the
//! ULA (`docs/01-core-emulator-spec.md` §3).

pub mod host;
pub mod keyboard;
pub mod memory;
pub mod sdk;
mod serialize;
pub mod snapshot;
pub mod tape;
pub mod ula;

use keyboard::{KeyPos, Keyboard};
use memory::Memory;
use tape::{Tape, TapeSignal};
use ula::Ula;
use z80::{Bus, Cpu, StopReason};

/// T-states in one 48K PAL frame (one `/INT` cycle).
pub const TSTATES_PER_FRAME: u32 = 69888;

/// 48K PAL frame rate: 3.5 MHz / 69888 T-states ≈ 50.08 Hz.
pub const FRAMES_PER_SEC: f64 = 3_500_000.0 / TSTATES_PER_FRAME as f64;

/// Frames to run before auto-loading a tape — enough to reach the BASIC `K` cursor.
pub const BOOT_FRAMES: u32 = 250;

/// The media format identifiers, so callers pass named constants rather than bare
/// strings (`load_media`/`load_snapshot`/`play_tape` all speak these).
pub mod format {
    pub const TAP: &str = "tap";
    pub const TZX: &str = "tzx";
    pub const SNA: &str = "sna";
    pub const Z80: &str = "z80";
    /// Every recognised media format.
    pub const ALL: &[&str] = &[TAP, TZX, SNA, Z80];
}

/// The media format for a filename by extension, for [`Spectrum::load_media`].
/// Case-insensitive; `None` if unrecognised. Returns a [`format`] constant.
pub fn media_format(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    format::ALL
        .iter()
        .copied()
        .find(|f| lower.ends_with(&format!(".{f}")))
}

/// Everything the CPU can touch: memory, the ULA clock/video, and the keyboard.
/// This is the `Bus` the CPU borrows.
pub struct Board {
    pub mem: Memory,
    pub ula: Ula,
    pub kb: Keyboard,
    /// A real-time tape signal (turbo/custom loaders); `None` = no tape playing.
    /// Advanced in lockstep with the ULA clock; drives the EAR input bit.
    pub tape_signal: Option<TapeSignal>,
    /// Optional host-trap dispatcher (`ED FE`); `None` = traps are NOPs.
    host: Option<Box<dyn host::HostCalls>>,
}

impl Board {
    /// Advance the real-time tape by `cycles` master T-states and reflect its
    /// EAR level into the keyboard port (bit 6). Called wherever the ULA clock
    /// advances, so the tape stays in sync with the CPU.
    fn advance_tape(&mut self, cycles: u32) {
        if let Some(t) = &mut self.tape_signal {
            if t.playing() {
                t.advance(cycles);
                self.kb.ear = t.level();
            }
        }
    }
}

impl Bus for Board {
    // The bus methods are pure data access — the CPU is the single source of
    // T-state accounting (it calls `tick`/`contend`). Keeping these side-effect
    // free is what lets the same opcode timings hold for both this Board and the
    // `FlatBus` test double.
    fn read(&mut self, addr: u16) -> u8 {
        self.mem.read(addr)
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.mem.write(addr, val);
    }

    fn input(&mut self, port: u16) -> u8 {
        if port & 0x0001 == 0 {
            // ULA port 0xFE: keyboard rows (active-low) in bits 0..4, EAR in 6.
            self.kb.read(port)
        } else {
            // TODO: floating bus / other ports. 0xFF is the safe default.
            0xFF
        }
    }

    fn output(&mut self, port: u16, val: u8) {
        if port & 0x0001 == 0 {
            // Port 0xFE: border (bits 0..2), MIC (bit 3), beeper (bit 4).
            self.ula.write_port_fe(val);
        }
    }

    fn contend(&mut self, addr: u16, _cycles: u32) {
        // The ULA stalls the CPU before a contended-memory access by an amount
        // that depends on the current frame T-state (M7). Pure data accesses to
        // ROM / upper RAM contend by 0.
        let delay = self.ula.contention(addr);
        if delay > 0 {
            self.ula.tick(delay);
            self.advance_tape(delay);
        }
    }

    fn tick(&mut self, cycles: u32) {
        self.ula.tick(cycles);
        self.advance_tape(cycles);
    }

    fn host_trap(&mut self, regs: &mut z80::Regs) -> u32 {
        // Split borrow: `host` and `mem` are disjoint fields, so the dispatcher
        // can read/write memory while it runs.
        match self.host.as_mut() {
            Some(h) => {
                let mut ctx = host::HostCtx::new(regs, &mut self.mem, &self.kb);
                h.dispatch(&mut ctx)
            }
            None => 0,
        }
    }
}

/// A whole 48K Spectrum.
pub struct Spectrum {
    pub cpu: Cpu,
    pub board: Board,
    /// A loaded `.tap`, fast-loaded by trapping the ROM (`None` = no tape).
    pub tape: Option<Tape>,
    /// Session recording: when on, every Nth frame's *indexed* screen is appended
    /// here (a raw observation; the host applies the palette and encodes video).
    rec_enabled: bool,
    rec_decimate: u32,
    rec_phase: u32,
    rec_count: u32,
    rec_indexed: Vec<u8>,
}

/// One disassembled instruction: its address, the raw bytes it spans, and the
/// mnemonic. Returned by [`Spectrum::disassemble`].
#[derive(Debug, Clone)]
pub struct DisasmLine {
    pub addr: u16,
    pub bytes: Vec<u8>,
    pub text: String,
}

impl Spectrum {
    /// Build a 48K machine. `rom` should be the 16K system ROM image; pass an
    /// empty slice to start with a blank ROM (useful for unit tests).
    pub fn new_48k(rom: &[u8]) -> Self {
        let mut cpu = Cpu::new();
        cpu.reset(); // canonical power-on state (PC=0, SP/AF=0xFFFF)
        Self {
            cpu,
            board: Board {
                mem: Memory::new_48k(rom),
                ula: Ula::new(),
                kb: Keyboard::new(),
                tape_signal: None,
                host: None,
            },
            tape: None,
            rec_enabled: false,
            rec_decimate: 1,
            rec_phase: 0,
            rec_count: 0,
            rec_indexed: Vec::new(),
        }
    }

    /// Execute a single instruction.
    pub fn step(&mut self) {
        self.cpu.step(&mut self.board);
    }

    /// Run one full frame. The ULA asserts `/INT` at the top of each frame, so we
    /// offer the interrupt first (the CPU accepts it at the boundary iff enabled),
    /// then execute one frame's worth of T-states, carrying any overshoot into the
    /// next frame. When a tape is loaded, the ROM `LD-BYTES` routine is trapped so
    /// blocks load instantly.
    pub fn run_frame(&mut self) -> StopReason {
        self.cpu.interrupt(&mut self.board);
        while self.board.ula.tstate < TSTATES_PER_FRAME {
            if self.tape.is_some() && self.cpu.regs.pc == tape::LD_BYTES {
                self.tape_trap();
                continue;
            }
            self.cpu.step(&mut self.board);
        }
        self.board
            .ula
            .finish_frame_audio(TSTATES_PER_FRAME, FRAMES_PER_SEC);
        self.board.ula.tstate -= TSTATES_PER_FRAME;
        self.board.ula.end_frame();

        // Session capture: append every `rec_decimate`-th frame's indexed screen.
        if self.rec_enabled {
            if self.rec_phase == 0 {
                let frame = self.board.ula.screen_indexed(self.board.mem.ram());
                self.rec_indexed.extend_from_slice(&frame);
                self.rec_count += 1;
            }
            self.rec_phase = (self.rec_phase + 1) % self.rec_decimate;
        }
        StopReason::Completed
    }

    // --- session recording ---------------------------------------------------

    /// Start capturing every `decimate`-th frame's indexed screen (audio is
    /// captured separately via `enable_audio`/`drain_audio`).
    pub fn start_recording(&mut self, decimate: u32) {
        self.rec_enabled = true;
        self.rec_decimate = decimate.max(1);
        self.rec_phase = 0;
        self.rec_count = 0;
        self.rec_indexed.clear();
    }

    /// Stop capturing; the buffered frames remain until taken.
    pub fn stop_recording(&mut self) {
        self.rec_enabled = false;
    }

    /// Number of frames captured, and the decimation used.
    pub fn recording_count(&self) -> u32 {
        self.rec_count
    }
    pub fn recording_decimate(&self) -> u32 {
        self.rec_decimate
    }

    /// Take the captured frames (flattened indexed screens, 256×192 bytes each).
    pub fn take_recording(&mut self) -> Vec<u8> {
        self.rec_count = 0;
        core::mem::take(&mut self.rec_indexed)
    }

    /// Enable beeper audio at the host sample rate. After each `run_frame`, call
    /// `drain_audio` to collect that frame's mono `f32` samples.
    pub fn enable_audio(&mut self, sample_rate: u32) {
        self.board.ula.enable_audio(sample_rate);
    }

    /// Take the audio samples produced since the last drain.
    pub fn drain_audio(&mut self) -> Vec<f32> {
        self.board.ula.drain_audio()
    }

    // --- raw observations (the `display` pipeline / MCP head consume these) ---

    /// The screen as logical colour indices (0–15), one per pixel. The raw
    /// observation the `display` crate themes.
    pub fn screen_indexed(&self) -> Vec<u8> {
        self.board.ula.screen_indexed(self.board.mem.ram())
    }

    /// The screen baked through the authentic palette (256×192×4 RGBA), no border.
    pub fn screen_rgba(&self) -> Vec<u8> {
        self.board.ula.render_rgba(self.board.mem.ram())
    }

    /// Current border colour (0–7).
    pub fn border(&self) -> u8 {
        self.board.ula.border
    }

    /// Read `len` bytes of the 64K address space starting at `addr`.
    pub fn read_memory(&self, addr: u16, len: u16) -> Vec<u8> {
        (0..len)
            .map(|i| self.board.mem.read(addr.wrapping_add(i)))
            .collect()
    }

    /// Write bytes into memory starting at `addr`. Writes to the ROM region are
    /// ignored, as on hardware.
    pub fn write_memory(&mut self, addr: u16, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.board.mem.write(addr.wrapping_add(i as u16), b);
        }
    }

    /// Disassemble `count` instructions from live memory starting at `addr`.
    /// Each line carries its address, raw bytes, and mnemonic; the next line
    /// begins at `addr + bytes.len()`.
    pub fn disassemble(&self, addr: u16, count: u16) -> Vec<DisasmLine> {
        let mut out = Vec::with_capacity(count as usize);
        let mut pc = addr;
        for _ in 0..count {
            let d = z80::disassemble(pc, |a| self.board.mem.read(a));
            let bytes = (0..d.len)
                .map(|i| self.board.mem.read(pc.wrapping_add(i as u16)))
                .collect();
            out.push(DisasmLine {
                addr: pc,
                bytes,
                text: d.text,
            });
            pc = pc.wrapping_add(d.len as u16);
        }
        out
    }

    // --- snapshots -----------------------------------------------------------

    /// Load a snapshot by format (`"sna"` or `"z80"`); the format is also sniffed
    /// from the data length if `fmt` is empty.
    pub fn load_snapshot(&mut self, fmt: &str, data: &[u8]) -> Result<(), snapshot::SnapshotError> {
        match fmt {
            format::SNA => snapshot::load_sna(self, data),
            format::Z80 => snapshot::load_z80(self, data),
            _ => {
                // Sniff: a 48K .sna is exactly 27 + 49152 bytes.
                if data.len() == 27 + 49152 {
                    snapshot::load_sna(self, data)
                } else {
                    snapshot::load_z80(self, data)
                }
            }
        }
    }

    /// Save the full machine state as a 48K `.sna`.
    pub fn save_sna(&self) -> Vec<u8> {
        snapshot::save_sna(self)
    }

    // --- tape (.tap fast-load) -----------------------------------------------

    /// Insert a `.tap`. Loading happens via the ROM trap once the program issues
    /// a tape read (e.g. after `LOAD ""`); see [`autoload_tape`](Self::autoload_tape).
    pub fn load_tap(&mut self, data: &[u8]) -> Result<(), snapshot::SnapshotError> {
        self.tape = Some(Tape::from_tap(data)?);
        Ok(())
    }

    /// Type `LOAD ""` + ENTER at the BASIC `K` cursor to start the tape (the `J`
    /// key is the `LOAD` keyword; `"` is SYM+P). Assumes the machine has booted to
    /// the prompt.
    pub fn autoload_tape(&mut self) {
        self.type_text("j\"\"\n");
    }

    /// Insert a tape for **real-time** (signal-level) loading and start playback.
    /// Unlike [`load_tap`](Self::load_tap)'s ROM trap, this drives the EAR line
    /// edge by edge, so turbo/custom loaders (most `.tzx` games) work. It loads in
    /// real time — a pilot tone alone is ~2 s — so run the machine for a while.
    /// `fmt` is `"tap"` or `"tzx"`.
    pub fn play_tape(&mut self, fmt: &str, data: &[u8]) -> Result<(), snapshot::SnapshotError> {
        let mut sig = TapeSignal::from_bytes(fmt, data)?;
        sig.play();
        self.board.tape_signal = Some(sig);
        Ok(())
    }

    /// True while a real-time tape is still playing.
    pub fn tape_playing(&self) -> bool {
        self.board.tape_signal.as_ref().is_some_and(|t| t.playing())
    }

    /// Load media into the machine by `fmt`, the one place every head shares:
    /// - `"sna"`/`"z80"` — restore a snapshot (instant; no boot needed);
    /// - `"tap"` — boot to the prompt, insert the tape, and `LOAD ""` (fast ROM
    ///   trap; run frames afterwards to let it complete);
    /// - `"tzx"` — boot, `LOAD ""`, and play the tape *signal* in real time
    ///   (turbo/custom loaders).
    ///
    /// A failed/unknown load is a no-op (returns `Err` for tapes so heads can warn).
    pub fn load_media(&mut self, fmt: &str, data: &[u8]) -> Result<(), snapshot::SnapshotError> {
        match fmt {
            format::TAP => {
                for _ in 0..BOOT_FRAMES {
                    self.run_frame();
                }
                self.load_tap(data)?;
                self.autoload_tape();
                Ok(())
            }
            format::TZX => {
                for _ in 0..BOOT_FRAMES {
                    self.run_frame();
                }
                self.autoload_tape();
                self.play_tape(format::TZX, data)
            }
            _ => self.load_snapshot(fmt, data),
        }
    }

    // --- host traps (`ED FE`) -----------------------------------------------

    /// Install the dispatcher that answers `ED FE` host traps. Without one, the
    /// opcode is a clean NOP (so a hybrid binary degrades on bare hardware).
    pub fn set_host_dispatcher(&mut self, d: Box<dyn host::HostCalls>) {
        self.board.host = Some(d);
    }

    /// Remove the host dispatcher; `ED FE` reverts to a NOP.
    pub fn clear_host_dispatcher(&mut self) {
        self.board.host = None;
    }

    /// Service the ROM `LD-BYTES` trap: read the next tape block and inject it.
    /// Entry state mirrors the ROM: `A` = expected flag, carry = LOAD/VERIFY,
    /// `IX` = address, `DE` = length. On success carry is set and the loaded bytes
    /// land in memory; on any mismatch / exhaustion carry is cleared. Either way
    /// we `RET` to the caller.
    fn tape_trap(&mut self) {
        const CF: u8 = 0x01;
        let expected_flag = self.cpu.regs.a;
        let is_load = self.cpu.regs.f & CF != 0;
        let ix = self.cpu.regs.ix;
        let de = self.cpu.regs.de();

        let block = self.tape.as_mut().and_then(|t| t.next_block());
        let block = match block {
            Some(b) if b.len() >= 2 => b,
            _ => return self.tape_return(false),
        };
        // Block layout: flag, data..., checksum.
        if block[0] != expected_flag {
            return self.tape_return(false);
        }
        let data = &block[1..block.len() - 1];
        let count = (de as usize).min(data.len());
        if is_load {
            for (i, &byte) in data[..count].iter().enumerate() {
                self.board.mem.write(ix.wrapping_add(i as u16), byte);
            }
        }
        self.cpu.regs.ix = ix.wrapping_add(count as u16);
        self.cpu.regs.set_de(de - count as u16);
        // Success only if we satisfied the full requested length.
        self.tape_return(count == de as usize);
    }

    /// Set carry to `success`, then `RET` (pop PC off the stack).
    fn tape_return(&mut self, success: bool) {
        if success {
            self.cpu.regs.f |= 0x01;
        } else {
            self.cpu.regs.f &= !0x01;
        }
        let sp = self.cpu.regs.sp;
        let lo = self.board.mem.read(sp) as u16;
        let hi = self.board.mem.read(sp.wrapping_add(1)) as u16;
        self.cpu.regs.pc = lo | (hi << 8);
        self.cpu.regs.sp = sp.wrapping_add(2);
    }

    // --- input (matrix-level, shared by every head's input table) ------------

    /// Set a key's pressed state directly in the matrix.
    pub fn set_key(&mut self, pos: KeyPos, pressed: bool) {
        self.board
            .kb
            .set_key(pos.row as usize, pos.col as usize, pressed);
    }

    /// Release every key (heads that get real key-up events rebuild the matrix
    /// from the currently-held set each frame).
    pub fn clear_keys(&mut self) {
        self.board.kb.release_all();
    }

    /// Press a key (optionally with CAPS/SYM shift held), letting the ROM's
    /// interrupt-driven scan see it: hold for `hold` frames, then release and run
    /// `gap` frames so the next press registers as new.
    pub fn press(&mut self, shift: Option<KeyPos>, pos: KeyPos, hold: u32, gap: u32) {
        if let Some(s) = shift {
            self.set_key(s, true);
        }
        self.set_key(pos, true);
        for _ in 0..hold {
            self.run_frame();
        }
        self.set_key(pos, false);
        if let Some(s) = shift {
            self.set_key(s, false);
        }
        for _ in 0..gap {
            self.run_frame();
        }
    }

    /// Type a string by resolving each character through the keyboard table and
    /// pressing it. Characters with no mapping are skipped. Returns how many were
    /// typed. `\n` is ENTER.
    pub fn type_text(&mut self, text: &str) -> usize {
        let mut typed = 0;
        for ch in text.chars() {
            if let Some((pos, caps, sym)) = keyboard::key_for_char(ch) {
                let shift = if caps {
                    Some(keyboard::CAPS_SHIFT)
                } else if sym {
                    Some(keyboard::SYM_SHIFT)
                } else {
                    None
                };
                // Hold a few frames, then release for longer: the ROM only
                // registers a key as *new* after it has scanned it released, so a
                // generous gap is needed when the same key repeats (e.g. `""`).
                self.press(shift, pos, 3, 9);
                typed += 1;
            }
        }
        typed
    }

    /// Decode the 32×24 text screen by matching each 8×8 cell against the system
    /// font in ROM (0x3D00, ASCII 32–127). Near-free, exact for ROM-printed text,
    /// and the cheapest way to confirm the machine booted — see the MCP spec's
    /// `read_screen_text`. Unmatched non-blank cells become `?`.
    pub fn screen_text(&self) -> String {
        const FONT_BASE: u16 = 0x3D00;
        let mut out = String::with_capacity(24 * 33);
        for row in 0..24usize {
            for col in 0..32usize {
                let mut cell = [0u8; 8];
                for (pl, byte) in cell.iter_mut().enumerate() {
                    let addr = 0x4000 + Ula::pixel_row_addr(row * 8 + pl, col) as u16;
                    *byte = self.board.mem.read(addr);
                }
                out.push(self.match_glyph(&cell, FONT_BASE));
            }
            out.push('\n');
        }
        out
    }

    fn match_glyph(&self, cell: &[u8; 8], font_base: u16) -> char {
        if cell.iter().all(|&b| b == 0) {
            return ' ';
        }
        for c in 32u8..128 {
            let base = font_base + (c as u16 - 32) * 8;
            if (0..8).all(|i| self.board.mem.read(base + i as u16) == cell[i as usize]) {
                return c as char;
            }
        }
        '?'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sna_roundtrip_preserves_state() {
        // ROM-free: set arbitrary state, save, load into a fresh machine, compare.
        let mut a = Spectrum::new_48k(&[]);
        a.cpu.regs.pc = 0x8000;
        a.cpu.regs.sp = 0xFF00;
        a.cpu.regs.set_bc(0x1234);
        a.cpu.regs.set_de(0x5678);
        a.cpu.regs.set_hl(0x9ABC);
        a.cpu.regs.a = 0x42;
        a.cpu.regs.f = 0x80;
        a.cpu.regs.ix = 0xCAFE;
        a.cpu.regs.iy = 0xBEEF;
        a.cpu.im = 2;
        a.cpu.iff1 = true;
        a.cpu.iff2 = true;
        a.board.ula.border = 3;
        a.write_memory(0x8000, &[0xDE, 0xAD, 0xBE, 0xEF]);

        let snap = a.save_sna();
        assert_eq!(snap.len(), 27 + 49152);

        let mut b = Spectrum::new_48k(&[]);
        b.load_snapshot(format::SNA, &snap).expect("load");

        assert_eq!(b.cpu.regs.pc, 0x8000, "PC popped from stack");
        assert_eq!(b.cpu.regs.sp, 0xFF00, "SP restored");
        assert_eq!(b.cpu.regs.bc(), 0x1234);
        assert_eq!(b.cpu.regs.de(), 0x5678);
        assert_eq!(b.cpu.regs.hl(), 0x9ABC);
        assert_eq!(b.cpu.regs.a, 0x42);
        assert_eq!(b.cpu.regs.ix, 0xCAFE);
        assert_eq!(b.cpu.regs.iy, 0xBEEF);
        assert_eq!(b.cpu.im, 2);
        assert!(b.cpu.iff1 && b.cpu.iff2);
        assert_eq!(b.board.ula.border, 3);
        assert_eq!(b.read_memory(0x8000, 4), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn tap_trap_injects_block() {
        // One data block: flag 0xFF, 4 data bytes, XOR checksum.
        let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let mut block = vec![0xFFu8];
        block.extend_from_slice(&payload);
        block.push(block.iter().fold(0, |a, &b| a ^ b));
        let mut tap = (block.len() as u16).to_le_bytes().to_vec();
        tap.extend_from_slice(&block);

        let mut spec = Spectrum::new_48k(&[]);
        spec.load_tap(&tap).unwrap();
        // Entry state as the ROM sets up before calling LD-BYTES.
        spec.cpu.regs.a = 0xFF; // expect a data block
        spec.cpu.regs.f = 0x01; // carry set => LOAD
        spec.cpu.regs.ix = 0x8000; // destination
        spec.cpu.regs.set_de(4); // length
        spec.cpu.regs.sp = 0xFF00;
        spec.write_memory(0xFF00, &[0x34, 0x12]); // return address 0x1234
        spec.cpu.regs.pc = tape::LD_BYTES;

        spec.tape_trap();

        assert_eq!(spec.read_memory(0x8000, 4), payload, "block injected");
        assert_eq!(spec.cpu.regs.pc, 0x1234, "RET to caller");
        assert_eq!(spec.cpu.regs.sp, 0xFF02, "stack popped");
        assert_eq!(spec.cpu.regs.f & 0x01, 0x01, "carry set = success");
        assert_eq!(spec.cpu.regs.ix, 0x8004, "IX advanced past the load");
        assert_eq!(spec.cpu.regs.de(), 0, "DE counted down to 0");
    }

    #[test]
    fn tap_trap_flag_mismatch_fails() {
        // Expecting a header (0x00) but the tape has a data block (0xFF).
        let mut block = vec![0xFFu8, 1, 2, 3];
        block.push(block.iter().fold(0, |a, &b| a ^ b));
        let mut tap = (block.len() as u16).to_le_bytes().to_vec();
        tap.extend_from_slice(&block);

        let mut spec = Spectrum::new_48k(&[]);
        spec.load_tap(&tap).unwrap();
        spec.cpu.regs.a = 0x00; // expecting a header
        spec.cpu.regs.f = 0x01;
        spec.cpu.regs.set_de(3);
        spec.cpu.regs.sp = 0xFF00;
        spec.write_memory(0xFF00, &[0x00, 0x10]); // return 0x1000
        spec.cpu.regs.pc = tape::LD_BYTES;

        spec.tape_trap();
        assert_eq!(
            spec.cpu.regs.f & 0x01,
            0,
            "carry clear = failure on mismatch"
        );
        assert_eq!(spec.cpu.regs.pc, 0x1000, "still RETs");
    }

    #[test]
    fn contention_stalls_bottom_16k_during_display() {
        // The same NOP stream costs more T-states when executed from contended
        // RAM (0x4000) during the display area than from clean RAM (0x8000).
        fn elapsed_from(addr: u16, tstate: u32) -> u32 {
            let mut spec = Spectrum::new_48k(&[]);
            spec.write_memory(addr, &[0x00; 64]); // NOPs
            spec.cpu.regs.pc = addr;
            spec.board.ula.tstate = tstate;
            let start = spec.board.ula.tstate;
            for _ in 0..50 {
                spec.step();
            }
            spec.board.ula.tstate - start
        }
        // Inside the display-fetch window (>14335): contention applies.
        let contended = elapsed_from(0x4000, 20_000);
        let clean = elapsed_from(0x8000, 20_000);
        assert_eq!(clean, 200, "50 uncontended NOPs are exactly 4T each");
        assert!(
            contended > clean,
            "contended {contended} should exceed clean {clean}"
        );
        // Outside the display area (e.g. vblank) even bottom-16K code isn't stalled.
        let border = elapsed_from(0x4000, 0);
        assert_eq!(border, 200, "no contention outside the display window");
    }

    #[test]
    fn recording_captures_decimated_frames() {
        let mut spec = Spectrum::new_48k(&[]);
        spec.start_recording(2); // every 2nd frame
        for _ in 0..10 {
            spec.run_frame();
        }
        spec.stop_recording();
        assert_eq!(
            spec.recording_count(),
            5,
            "10 frames / decimate 2 = 5 captured"
        );
        let data = spec.take_recording();
        assert_eq!(data.len(), 5 * 256 * 192, "flattened indexed screens");
        // Buffer is drained.
        assert_eq!(spec.recording_count(), 0);
        // Stopped: further frames aren't captured.
        spec.run_frame();
        assert_eq!(spec.recording_count(), 0);
    }

    #[test]
    fn disassembles_from_memory() {
        let mut spec = Spectrum::new_48k(&[]);
        // LD HL,$4000 ; INC HL ; LD A,(HL) ; HALT
        spec.write_memory(0x8000, &[0x21, 0x00, 0x40, 0x23, 0x7E, 0x76]);
        let lines = spec.disassemble(0x8000, 4);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["LD HL,$4000", "INC HL", "LD A,(HL)", "HALT"]);
        assert_eq!(lines[0].addr, 0x8000);
        assert_eq!(lines[0].bytes, vec![0x21, 0x00, 0x40]);
        assert_eq!(
            lines[1].addr, 0x8003,
            "next line follows the previous length"
        );
    }

    #[test]
    fn host_trap_dispatches_to_fntable() {
        use crate::host::{FnTable, HostCtx};
        let mut spec = Spectrum::new_48k(&[]);
        let mut table = FnTable::new();
        // 0x10 = mul16: HL = BC * DE.
        table.on(0x10, |ctx: &mut HostCtx| {
            let bc = (ctx.regs.b as u16) << 8 | ctx.regs.c as u16;
            let de = (ctx.regs.d as u16) << 8 | ctx.regs.e as u16;
            ctx.regs.set_hl(bc.wrapping_mul(de));
            ctx.ok();
            0
        });
        spec.set_host_dispatcher(Box::new(table));

        spec.write_memory(0x8000, &[0xED, 0xFE]); // HOSTCALL
        spec.cpu.regs.pc = 0x8000;
        spec.cpu.regs.a = 0x10;
        spec.cpu.regs.set_bc(7);
        spec.cpu.regs.set_de(6);
        spec.step();

        assert_eq!(spec.cpu.regs.hl(), 42, "BC*DE written back to HL");
        assert_eq!(spec.cpu.regs.pc, 0x8002, "two-byte opcode consumed");
        assert!(!spec.cpu.regs.carry(), "ok() cleared carry");
    }

    #[test]
    fn host_trap_unknown_id_sets_carry() {
        let mut spec = Spectrum::new_48k(&[]);
        spec.set_host_dispatcher(Box::new(crate::host::FnTable::new()));
        spec.write_memory(0x8000, &[0xED, 0xFE]);
        spec.cpu.regs.pc = 0x8000;
        spec.cpu.regs.a = 0x99; // no handler registered
        spec.cpu.regs.set_carry(false);
        spec.step();
        assert!(spec.cpu.regs.carry(), "unknown id fails with CF=1");
        assert_eq!(spec.cpu.regs.pc, 0x8002);
    }

    #[test]
    fn math_traps_mul_and_divmod() {
        let mut spec = Spectrum::new_48k(&[]);
        spec.set_host_dispatcher(Box::new(host::math_traps()));
        let run = |spec: &mut Spectrum, a: u8, bc: u16, de: u16| {
            spec.write_memory(0x9000, &[0xED, 0xFE]);
            spec.cpu.regs.pc = 0x9000;
            spec.cpu.regs.a = a;
            spec.cpu.regs.set_bc(bc);
            spec.cpu.regs.set_de(de);
            spec.step();
        };
        run(&mut spec, 0x10, 200, 50); // MUL16: 200*50
        assert_eq!(spec.cpu.regs.hl(), 10_000);
        assert!(!spec.cpu.regs.carry());

        run(&mut spec, 0x11, 1000, 7); // DIVMOD16: 1000/7
        assert_eq!(spec.cpu.regs.hl(), 142);
        assert_eq!(spec.cpu.regs.de(), 6);

        run(&mut spec, 0x11, 5, 0); // divide by zero → carry
        assert!(spec.cpu.regs.carry());
    }

    #[test]
    fn host_trap_is_nop_without_dispatcher() {
        // The fidelity dial: with no host, ED FE does nothing (as on real silicon).
        let mut spec = Spectrum::new_48k(&[]);
        spec.write_memory(0x8000, &[0xED, 0xFE]);
        spec.cpu.regs.pc = 0x8000;
        spec.cpu.regs.set_hl(0);
        spec.cpu.regs.a = 0x00; // HOST_PRESENT probe
        spec.step();
        assert_eq!(spec.cpu.regs.hl(), 0, "HL unchanged → bare hardware");
        assert_eq!(spec.cpu.regs.pc, 0x8002);
    }

    #[test]
    fn beeper_audio_renders_a_tone() {
        // A square wave toggled mid-frame should produce samples spanning roughly
        // the full -AMP..+AMP range; a silent frame should be flat.
        let mut spec = Spectrum::new_48k(&[]);
        spec.enable_audio(44_100);

        // Silent frame (no port writes): all samples equal, near the rest level.
        spec.run_frame();
        let silent = spec.drain_audio();
        assert!(!silent.is_empty(), "produced samples");
        assert!(
            silent.iter().all(|&s| (s - silent[0]).abs() < 1e-6),
            "silent frame is flat"
        );

        // Now toggle the speaker many times across a frame -> a real waveform.
        // Drive the bus directly: write 0xFE with bit 4 flipping, spaced in time.
        spec.enable_audio(44_100);
        for i in 0..200u32 {
            spec.board.ula.tick(300); // advance ~300 T-states between toggles
            let level = if i % 2 == 0 { 0x10 } else { 0x00 };
            spec.board.ula.write_port_fe(level);
        }
        spec.board
            .ula
            .finish_frame_audio(TSTATES_PER_FRAME, FRAMES_PER_SEC);
        let tone = spec.drain_audio();
        let max = tone.iter().cloned().fold(f32::MIN, f32::max);
        let min = tone.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.1, "tone reaches high (max={max})");
        assert!(min < -0.1, "tone reaches low (min={min})");
    }

    #[test]
    fn pixel_row_addr_known_values() {
        // Regression for the interleaved-thirds screen layout. The "which third"
        // bits select a 2 KB block, not a small offset.
        let a = Ula::pixel_row_addr;
        assert_eq!(a(0, 0), 0x0000, "top-left");
        assert_eq!(a(0, 31), 0x001F, "top row, last column");
        assert_eq!(a(1, 0), 0x0100, "next pixel line within a char is +256");
        assert_eq!(a(8, 0), 0x0020, "next char row is +32");
        assert_eq!(a(64, 0), 0x0800, "middle third starts at +0x800");
        assert_eq!(a(128, 0), 0x1000, "bottom third starts at +0x1000");
        assert_eq!(a(191, 31), 0x17FF, "bottom-right is the last screen byte");
    }

    /// Boot the real 48K ROM and confirm it reaches the copyright prompt.
    /// Off by default (ROM not bundled). Cargo runs tests from the package dir,
    /// so use an absolute path:
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p spectrum -- --ignored boots
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn boots_to_copyright() {
        let path = std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM");
        let rom = std::fs::read(&path).expect("read ROM");
        let mut spec = Spectrum::new_48k(&rom);
        for _ in 0..250 {
            spec.run_frame();
        }
        let text = spec.screen_text();
        assert!(
            text.contains("1982 Sinclair Research Ltd"),
            "expected the boot copyright on screen, got:\n{text}"
        );
        assert!(spec.cpu.iff1, "ROM should have enabled interrupts");
        assert_eq!(spec.cpu.im, 1, "ROM should be in IM 1");
    }

    /// Load a game and confirm the beeper actually makes sound — Manic Miner's
    /// title screen plays a tune, so several seconds of frames should yield a
    /// non-trivial waveform (not silence).
    ///   SPECTRUM_ROM=... SPECTRUM_GAME="$PWD/testroms/manic.z80" \
    ///       cargo test -p spectrum -- --ignored title_music
    #[test]
    #[ignore = "set SPECTRUM_ROM and SPECTRUM_GAME to absolute paths"]
    fn title_music_makes_sound() {
        let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
        let game = std::fs::read(std::env::var("SPECTRUM_GAME").expect("SPECTRUM_GAME")).unwrap();
        let mut spec = Spectrum::new_48k(&rom);
        let fmt = media_format(&std::env::var("SPECTRUM_GAME").unwrap()).unwrap_or(format::Z80);
        spec.load_snapshot(fmt, &game).expect("load game");
        spec.enable_audio(44_100);

        let mut all = Vec::new();
        for _ in 0..150 {
            // ~3 seconds
            spec.run_frame();
            all.extend(spec.drain_audio());
        }
        let max = all.iter().cloned().fold(f32::MIN, f32::max);
        let min = all.iter().cloned().fold(f32::MAX, f32::min);
        let nonzero = all.iter().filter(|&&s| s.abs() > 0.01).count();
        assert!(
            max > 0.1 && min < -0.1 && nonzero > 1000,
            "title music should oscillate: max={max} min={min} nonzero={nonzero}"
        );
    }

    /// Build a 48K `.tap` holding one auto-running BASIC line `10 BORDER <n>`.
    fn make_basic_border_tap(n: u8) -> Vec<u8> {
        // Tokenised line body: BORDER <n> ENTER. A number is stored as its ASCII
        // digit followed by the hidden 0x0E + 5-byte integer form.
        let mut body = vec![0xE7u8, b'0' + n]; // BORDER token, ASCII digit
        body.extend_from_slice(&[0x0E, 0x00, 0x00, n, 0x00, 0x00]); // hidden number
        body.push(0x0D); // ENTER
        let mut prog = vec![0x00, 0x0A]; // line number 10, big-endian
        prog.extend_from_slice(&(body.len() as u16).to_le_bytes()); // line length
        prog.extend_from_slice(&body);
        let prog_len = prog.len() as u16;

        let mut hdr = vec![0u8; 17];
        hdr[0] = 0; // type: BASIC program
        hdr[1..11].copy_from_slice(b"BORDER    "); // 10-char filename
        hdr[11..13].copy_from_slice(&prog_len.to_le_bytes()); // data length
        hdr[13..15].copy_from_slice(&10u16.to_le_bytes()); // autostart line 10
        hdr[15..17].copy_from_slice(&prog_len.to_le_bytes()); // start of variables

        let mut tap = tap_block(0x00, &hdr);
        tap.extend(tap_block(0xFF, &prog));
        tap
    }

    /// Wrap `data` as a `.tap` block: `[u16 len][flag .. data .. xor-checksum]`.
    fn tap_block(flag: u8, data: &[u8]) -> Vec<u8> {
        let mut block = vec![flag];
        block.extend_from_slice(data);
        block.push(block.iter().fold(0u8, |a, &b| a ^ b));
        let mut out = (block.len() as u16).to_le_bytes().to_vec();
        out.extend(block);
        out
    }

    /// Full ROM tape flow: boot, `LOAD ""`, trap-load the header + program, and
    /// auto-run it. Proves the real ROM reaches `LD-BYTES` with the register state
    /// the trap assumes.
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p spectrum -- --ignored tap_loads
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn tap_loads_and_autoruns_basic() {
        let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();
        let mut spec = Spectrum::new_48k(&rom);
        for _ in 0..250 {
            spec.run_frame(); // boot to the K cursor
        }
        spec.load_tap(&make_basic_border_tap(6)).unwrap();
        spec.autoload_tape(); // types LOAD ""
        for _ in 0..300 {
            spec.run_frame(); // load both blocks via the trap, then auto-run
        }
        assert_eq!(
            spec.board.ula.border,
            6,
            "BORDER 6 should have run after the tape auto-loaded; screen:\n{}",
            spec.screen_text()
        );
        assert!(
            spec.tape.as_ref().unwrap().finished(),
            "both blocks consumed"
        );
    }

    /// Type a BASIC expression and confirm the ROM evaluates it. Exercises the
    /// keyboard matrix, interrupt-driven scan, tokeniser, and execution at once.
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p spectrum -- --ignored types
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn types_basic_and_evaluates() {
        let path = std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM");
        let rom = std::fs::read(&path).expect("read ROM");
        let mut spec = Spectrum::new_48k(&rom);
        for _ in 0..250 {
            spec.run_frame();
        }
        // P at the K cursor -> the PRINT keyword, then the expression.
        spec.press(None, keyboard::KeyPos { row: 5, col: 0 }, 3, 3);
        spec.type_text("6*7\n");
        for _ in 0..30 {
            spec.run_frame();
        }
        let text = spec.screen_text();
        assert!(
            text.lines().next().unwrap_or("").contains("42"),
            "expected 6*7=42 printed at the top, got:\n{text}"
        );
    }

    /// The interactive Z80 chat terminal: type a line, ENTER, and the host's
    /// reply is teletyped back onto the screen.
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p spectrum -- --ignored chat_terminal
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn chat_terminal_round_trip() {
        let path = std::env::var("SPECTRUM_ROM").expect("set SPECTRUM_ROM");
        let rom = std::fs::read(&path).expect("read ROM");
        let mut spec = Spectrum::new_48k(&rom);
        for _ in 0..250 {
            spec.run_frame();
        }
        spec.set_host_dispatcher(Box::new(host::chat_traps()));
        spec.write_memory(sdk::CHAT_TERMINAL_ORG, &sdk::CHAT_TERMINAL);
        spec.cpu.regs.pc = sdk::CHAT_TERMINAL_ORG;
        for _ in 0..3 {
            spec.run_frame(); // let the terminal's init run (it sets L-mode) before typing
        }

        // Type "hi" + ENTER through the keyboard matrix; the terminal reads it
        // from LAST-K (the ROM ISR scans the matrix each frame).
        spec.type_text("hi\n");
        for _ in 0..20 {
            spec.run_frame();
        }

        let text = spec.screen_text();
        assert!(
            text.contains("You said: HI") || text.contains("You said: hi"),
            "expected the host reply on screen, got:\n{text}"
        );
    }
}

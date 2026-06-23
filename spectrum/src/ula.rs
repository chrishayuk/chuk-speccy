//! The ULA: master T-state clock, port 0xFE state (border/beeper), and video
//! decode. Constants are 48K PAL (`docs/01-core-emulator-spec.md` §5).

use crate::TSTATES_PER_FRAME;

/// Frame T-state at which contention of the first display byte begins (48K). The
/// famous off-by-one debate lives here; 14335 is a widely-used value.
const FIRST_CONTENDED_TSTATE: u32 = 14335;
/// Stall added at each offset into an 8-T window while the ULA is fetching the
/// display (`docs/01-core-emulator-spec.md` §5.3).
const CONTENTION_PATTERN: [u8; 8] = [6, 5, 4, 3, 2, 1, 0, 0];

/// Build the per-frame `[u8; 69888]` contention table: for each of the 192
/// display lines, the first 128 T-states (the pixel-fetch period) carry the
/// 8-cycle stall pattern; everywhere else is 0.
fn build_contention_table() -> Vec<u8> {
    let mut table = vec![0u8; TSTATES_PER_FRAME as usize];
    for line in 0..192u32 {
        let line_start = FIRST_CONTENDED_TSTATE + line * 224;
        for x in 0..128u32 {
            let idx = (line_start + x) as usize;
            if idx < table.len() {
                table[idx] = CONTENTION_PATTERN[(x % 8) as usize];
            }
        }
    }
    table
}

/// The 8 hardware colours as RGBA, [normal, bright]. Index by the 3-bit colour.
/// Bright black is the same as normal black.
pub const PALETTE: [[u8; 4]; 16] = [
    // normal (BRIGHT=0)
    [0x00, 0x00, 0x00, 0xFF], // black
    [0x00, 0x00, 0xD7, 0xFF], // blue
    [0xD7, 0x00, 0x00, 0xFF], // red
    [0xD7, 0x00, 0xD7, 0xFF], // magenta
    [0x00, 0xD7, 0x00, 0xFF], // green
    [0x00, 0xD7, 0xD7, 0xFF], // cyan
    [0xD7, 0xD7, 0x00, 0xFF], // yellow
    [0xD7, 0xD7, 0xD7, 0xFF], // white
    // bright (BRIGHT=1)
    [0x00, 0x00, 0x00, 0xFF],
    [0x00, 0x00, 0xFF, 0xFF],
    [0xFF, 0x00, 0x00, 0xFF],
    [0xFF, 0x00, 0xFF, 0xFF],
    [0x00, 0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF, 0xFF],
    [0xFF, 0xFF, 0x00, 0xFF],
    [0xFF, 0xFF, 0xFF, 0xFF],
];

pub const SCREEN_W: usize = 256;
pub const SCREEN_H: usize = 192;

pub struct Ula {
    /// Frame T-state position. Wraps each frame; the master clock for timing.
    pub tstate: u32,
    /// Frame counter, for FLASH (swaps ink/paper every 16 frames: `& 0x10`).
    pub frame: u32,
    /// Border colour (0..7), from the low 3 bits of port 0xFE writes.
    pub border: u8,
    /// Last beeper bit (bit 4 of port 0xFE). Sampled against the clock for audio.
    pub beeper: bool,

    // --- audio (beeper) ---
    /// When enabled, beeper edges are recorded and rendered to samples per frame.
    audio_enabled: bool,
    /// Host sample rate (Hz).
    audio_rate: u32,
    /// Carry of fractional samples between frames, to track the host rate exactly.
    audio_acc: f64,
    /// Speaker level at the start of the current frame.
    audio_start_level: bool,
    /// Beeper toggles this frame: (frame T-state, new level).
    audio_edges: Vec<(u32, bool)>,
    /// Rendered samples awaiting the host (drained by the frontend).
    audio_out: Vec<f32>,

    /// Precomputed per-frame contention stalls, indexed by frame T-state.
    contention: Vec<u8>,
    /// When false, contention is bypassed (for A/B timing comparisons).
    pub contention_enabled: bool,
}

impl Ula {
    /// Append the execution-relevant ULA state to a full-state blob: the frame
    /// phase, border, beeper, and audio carry. The `contention` table is a constant
    /// (rebuilt by `new`); the per-frame audio edge/output buffers are transient
    /// (regenerated each frame) and don't affect future CPU/video state.
    pub(crate) fn save(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.tstate.to_le_bytes());
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.push(self.border);
        out.push(self.beeper as u8);
        out.push(self.contention_enabled as u8);
        out.push(self.audio_enabled as u8);
        out.extend_from_slice(&self.audio_rate.to_le_bytes());
        out.extend_from_slice(&self.audio_acc.to_le_bytes());
        out.push(self.audio_start_level as u8);
    }
    /// Restore the ULA state from a blob cursor (inverse of [`save`](Self::save)).
    pub(crate) fn load(&mut self, c: &mut crate::serialize::Cur) {
        self.tstate = c.u32();
        self.frame = c.u32();
        self.border = c.u8();
        self.beeper = c.bool();
        self.contention_enabled = c.bool();
        self.audio_enabled = c.bool();
        self.audio_rate = c.u32();
        self.audio_acc = c.f64();
        self.audio_start_level = c.bool();
    }

    pub fn new() -> Self {
        Self {
            tstate: 0,
            frame: 0,
            border: 0,
            beeper: false,
            audio_enabled: false,
            audio_rate: 0,
            audio_acc: 0.0,
            audio_start_level: false,
            audio_edges: Vec::new(),
            audio_out: Vec::new(),
            contention: build_contention_table(),
            contention_enabled: true,
        }
    }

    /// Advance the master clock.
    #[inline]
    pub fn tick(&mut self, cycles: u32) {
        self.tstate += cycles;
    }

    /// Stall (in T-states) the ULA imposes on an access to `addr` *right now*.
    /// Contention only applies to the bottom-16K RAM (`0x4000–0x7FFF`) during the
    /// display-fetch windows; everything else is 0.
    #[inline]
    pub fn contention(&self, addr: u16) -> u32 {
        if self.contention_enabled && (0x4000..0x8000).contains(&addr) {
            let idx = (self.tstate % TSTATES_PER_FRAME) as usize;
            self.contention[idx] as u32
        } else {
            0
        }
    }

    /// Handle a write to port 0xFE.
    #[inline]
    pub fn write_port_fe(&mut self, val: u8) {
        self.border = val & 0x07;
        let level = val & 0x10 != 0;
        if level != self.beeper {
            self.beeper = level;
            if self.audio_enabled {
                self.audio_edges.push((self.tstate, level));
            }
        }
    }

    /// Turn on beeper audio at the given host sample rate.
    pub fn enable_audio(&mut self, sample_rate: u32) {
        self.audio_enabled = true;
        self.audio_rate = sample_rate;
        self.audio_acc = 0.0;
        self.audio_start_level = self.beeper;
        self.audio_edges.clear();
        self.audio_out.clear();
    }

    /// Render this frame's beeper waveform into the sample buffer (box-filter
    /// downsample), then reset edges for the next frame. `frame_t` is the frame
    /// length in T-states. Cheap no-op when audio is disabled.
    pub fn finish_frame_audio(&mut self, frame_t: u32, frames_per_sec: f64) {
        if !self.audio_enabled {
            self.audio_edges.clear();
            return;
        }
        // How many output samples this frame, carrying the fractional remainder.
        self.audio_acc += self.audio_rate as f64 / frames_per_sec;
        let n = self.audio_acc.floor() as usize;
        self.audio_acc -= n as f64;
        if n > 0 {
            render_beeper(
                &mut self.audio_out,
                &self.audio_edges,
                self.audio_start_level,
                n,
                frame_t,
            );
        }
        // The end-of-frame level becomes the next frame's starting level.
        self.audio_start_level = self.beeper;
        self.audio_edges.clear();
    }

    /// Take and clear the rendered audio samples (mono `f32`, -1.0..1.0).
    pub fn drain_audio(&mut self) -> Vec<f32> {
        core::mem::take(&mut self.audio_out)
    }

    /// Called at frame end: bump the frame counter (drives FLASH).
    #[inline]
    pub fn end_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// True when FLASH cells should currently show swapped ink/paper.
    #[inline]
    pub fn flash_on(&self) -> bool {
        self.frame & 0x10 != 0
    }

    /// Map a pixel row (0..191) and byte column (0..31) to the screen-memory byte
    /// offset within RAM (relative to 0x4000). The infamous interleaved-thirds
    /// layout: the 13-bit offset is `Y7 Y6 | Y2 Y1 Y0 | Y5 Y4 Y3 | X4..X0`, i.e.
    /// the third (`y & 0xC0`) lands at bits 11–12, the pixel-row-within-char
    /// (`y & 0x07`) at bits 8–10, and the char-row-within-third (`y & 0x38`) at
    /// bits 5–7.
    #[inline]
    pub fn pixel_row_addr(y: usize, x_byte: usize) -> usize {
        ((y & 0b1100_0000) << 5)        // which third -> bits 11-12
            | ((y & 0b0000_0111) << 8)  // pixel row within char -> bits 8-10
            | ((y & 0b0011_1000) << 2)  // char row within third -> bits 5-7
            | x_byte
    }

    /// Decode the current screen to a freshly allocated RGBA framebuffer
    /// (256*192*4). `ram` is the 48K RAM slice (screen at offset 0).
    ///
    /// Decode the screen to an *indexed* framebuffer: `SCREEN_W * SCREEN_H` bytes,
    /// one logical colour (0–15, bright = +8) per pixel. This is the raw
    /// observation the `display` pipeline themes; the core never bakes RGB.
    ///
    /// Scaffold: per-frame render from final memory state (correct for ~all
    /// games; multicolour demos want per-scanline — a localised upgrade later).
    pub fn screen_indexed(&self, ram: &[u8]) -> Vec<u8> {
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H];
        let flash = self.flash_on();
        for y in 0..SCREEN_H {
            for xb in 0..(SCREEN_W / 8) {
                let bits = ram[Self::pixel_row_addr(y, xb)];
                // Attributes are linear: 0x1800 offset (i.e. 0x5800 - 0x4000).
                let attr = ram[0x1800 + (y / 8) * 32 + xb];
                let (ink, paper) = decode_attr(attr, flash);
                for bit in 0..8 {
                    let lit = bits & (0x80 >> bit) != 0;
                    fb[y * SCREEN_W + xb * 8 + bit] = (if lit { ink } else { paper }) as u8;
                }
            }
        }
        fb
    }

    /// Convenience: the indexed screen mapped through the authentic palette. This
    /// is the `display` crate's `authentic` preset baked in for the simplest path
    /// (and the MCP `screen_rgba` baseline). 256×192×4, no border.
    pub fn render_rgba(&self, ram: &[u8]) -> Vec<u8> {
        let idx = self.screen_indexed(ram);
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        for (i, &c) in idx.iter().enumerate() {
            fb[i * 4..i * 4 + 4].copy_from_slice(&PALETTE[c as usize]);
        }
        fb
    }
}

impl Default for Ula {
    fn default() -> Self {
        Self::new()
    }
}

/// Peak beeper amplitude (kept well below 1.0 to leave headroom / avoid clipping).
const BEEPER_AMP: f32 = 0.22;

/// Box-filter a frame's beeper square wave into `n` samples appended to `out`.
/// Each output sample is the fraction of its T-state window the speaker was high,
/// mapped to `-AMP..+AMP`. `edges` are `(tstate, new_level)` in time order;
/// `start_level` is the speaker level before the first edge.
fn render_beeper(
    out: &mut Vec<f32>,
    edges: &[(u32, bool)],
    start_level: bool,
    n: usize,
    frame_t: u32,
) {
    let mut high = vec![0u64; n]; // high-time per output-sample window, in T-states
    let mut t0 = 0u32;
    let mut level = start_level;
    for &(et, new_level) in edges {
        let et = et.min(frame_t);
        if level && et > t0 {
            add_high(&mut high, frame_t, t0, et);
        }
        t0 = et;
        level = new_level;
    }
    if level && frame_t > t0 {
        add_high(&mut high, frame_t, t0, frame_t);
    }

    let ft = frame_t as u64;
    let nn = n as u64;
    for (b, &h) in high.iter().enumerate() {
        let bs = b as u64 * ft / nn;
        let be = (b as u64 + 1) * ft / nn;
        let width = (be - bs).max(1) as f32;
        let frac = h as f32 / width; // 0.0..1.0
        out.push((frac * 2.0 - 1.0) * BEEPER_AMP);
    }
}

/// Distribute the high interval `[s0, s1)` (T-states) across the sample windows.
fn add_high(high: &mut [u64], frame_t: u32, s0: u32, s1: u32) {
    let n = high.len() as u64;
    let ft = frame_t as u64;
    let (s0, s1) = (s0 as u64, s1 as u64);
    let first = s0 * n / ft;
    let last = ((s1 - 1) * n / ft).min(n - 1);
    for b in first..=last {
        let bs = b * ft / n;
        let be = (b + 1) * ft / n;
        let lo = s0.max(bs);
        let hi = s1.min(be);
        if hi > lo {
            high[b as usize] += hi - lo;
        }
    }
}

/// Decode an attribute byte into (ink, paper) palette indices, honouring BRIGHT
/// and the current FLASH phase. `FLASH(7) BRIGHT(6) PAPER(5..3) INK(2..0)`.
#[inline]
fn decode_attr(attr: u8, flash_on: bool) -> (usize, usize) {
    let bright = (attr & 0x40) >> 3; // 0 or 8 into the palette
    let mut ink = (attr & 0x07) as usize;
    let mut paper = ((attr >> 3) & 0x07) as usize;
    if attr & 0x80 != 0 && flash_on {
        core::mem::swap(&mut ink, &mut paper);
    }
    (ink + bright as usize, paper + bright as usize)
}

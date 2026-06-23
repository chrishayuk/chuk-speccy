//! Tape support, two ways. **Fast load** (`Tape`): trap the 48K ROM's `LD-BYTES`
//! routine at `0x0556` and splat the next block straight into memory — instant,
//! standard-loader `.tap` only (`docs/01-core-emulator-spec.md` §7). **Real-time
//! load** (`TapeSignal`, below): play the tape as an edge signal into the EAR
//! line so a game's own turbo/custom loader works (`.tap`/`.tzx`).

use crate::snapshot::SnapshotError;

/// The 48K ROM `LD-BYTES` entry point. With the standard ROM, every tape read
/// passes through here with: `A` = expected flag byte (0x00 header / 0xFF data),
/// carry = LOAD(1)/VERIFY(0), `IX` = destination, `DE` = byte count.
pub const LD_BYTES: u16 = 0x0556;

/// A parsed `.tap`: an ordered list of blocks, each `flag .. data .. checksum`.
pub struct Tape {
    blocks: Vec<Vec<u8>>,
    pos: usize,
}

impl Tape {
    /// Parse a `.tap`: repeated `[u16 little-endian length][length bytes]`.
    pub fn from_tap(data: &[u8]) -> Result<Self, SnapshotError> {
        let mut blocks = Vec::new();
        let mut i = 0;
        while i + 2 <= data.len() {
            let len = (data[i] as usize) | ((data[i + 1] as usize) << 8);
            i += 2;
            if len == 0 || i + len > data.len() {
                break;
            }
            blocks.push(data[i..i + len].to_vec());
            i += len;
        }
        if blocks.is_empty() {
            return Err(SnapshotError::Truncated);
        }
        Ok(Self { blocks, pos: 0 })
    }

    /// Take the next block (consuming it), or None when the tape is exhausted.
    pub fn next_block(&mut self) -> Option<Vec<u8>> {
        let b = self.blocks.get(self.pos)?.clone();
        self.pos += 1;
        Some(b)
    }

    /// Rewind to the first block.
    pub fn rewind(&mut self) {
        self.pos = 0;
    }

    /// True once every block has been read.
    pub fn finished(&self) -> bool {
        self.pos >= self.blocks.len()
    }
}

// --- real-time (pulse-level) tape ------------------------------------------
//
// The trap above only serves games that load via the standard ROM routine.
// Turbo/custom loaders (most commercial `.tzx` titles) read the EAR line edge by
// edge, so they need the actual tape *signal*. `TapeSignal` is that signal: a
// flat pulse stream (each value a duration in T-states; the EAR level flips at
// every boundary), advanced by the master clock and read back via port 0xFE
// bit 6 (`docs/01-core-emulator-spec.md` accuracy tail).

/// Master clock T-states per millisecond (3.5 MHz).
const T_PER_MS: u32 = 3500;

// Standard ROM tape timings (T-states).
const PILOT: u32 = 2168;
const SYNC1: u32 = 667;
const SYNC2: u32 = 735;
const ZERO: u32 = 855;
const ONE: u32 = 1710;
const PILOT_HEADER_PULSES: u32 = 8063; // flag byte < 128
const PILOT_DATA_PULSES: u32 = 3223; // flag byte >= 128

/// A real-time tape as a flat pulse stream. `level()` drives EAR (port 0xFE bit
/// 6); `advance()` is clocked by the ULA so a game's own loader reads the edges.
pub struct TapeSignal {
    pulses: Vec<u32>,
    idx: usize,
    in_pulse: u32,
    level: bool,
    playing: bool,
}

impl TapeSignal {
    /// Build from `.tap` or `.tzx` bytes (`fmt` = "tap" | "tzx").
    pub fn from_bytes(fmt: &str, data: &[u8]) -> Result<Self, SnapshotError> {
        let pulses = match fmt {
            "tzx" => tzx_pulses(data)?,
            _ => tap_pulses(data)?,
        };
        if pulses.is_empty() {
            return Err(SnapshotError::Truncated);
        }
        Ok(Self {
            pulses,
            idx: 0,
            in_pulse: 0,
            level: false,
            playing: false,
        })
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// The current EAR level (true = high).
    pub fn level(&self) -> bool {
        self.level
    }

    /// True while still playing and pulses remain.
    pub fn playing(&self) -> bool {
        self.playing && self.idx < self.pulses.len()
    }

    /// Advance by `t` master T-states, flipping the EAR level at each pulse
    /// boundary. Stops playing at the end of the stream.
    pub fn advance(&mut self, mut t: u32) {
        if !self.playing {
            return;
        }
        while self.idx < self.pulses.len() {
            let remaining = self.pulses[self.idx].saturating_sub(self.in_pulse);
            if t < remaining {
                self.in_pulse += t;
                return;
            }
            t -= remaining;
            self.idx += 1;
            self.in_pulse = 0;
            self.level = !self.level;
        }
        self.playing = false;
    }

    /// Total pulses and length in T-states (for progress / tests).
    pub fn pulse_count(&self) -> usize {
        self.pulses.len()
    }
    pub fn total_tstates(&self) -> u64 {
        self.pulses.iter().map(|&p| p as u64).sum()
    }
}

/// `.tap` → pulses: each block gets standard-ROM pilot/sync/bit timing.
fn tap_pulses(data: &[u8]) -> Result<Vec<u32>, SnapshotError> {
    let mut pulses = Vec::new();
    let mut i = 0;
    let mut blocks = 0;
    while i + 2 <= data.len() {
        let len = (data[i] as usize) | ((data[i + 1] as usize) << 8);
        i += 2;
        if len == 0 || i + len > data.len() {
            break;
        }
        let block = &data[i..i + len];
        encode_block(
            &mut pulses,
            block,
            PILOT,
            SYNC1,
            SYNC2,
            ZERO,
            ONE,
            pilot_for(block),
            8,
            1000,
        );
        i += len;
        blocks += 1;
    }
    if blocks == 0 {
        return Err(SnapshotError::Truncated);
    }
    Ok(pulses)
}

/// Pilot tone length for a standard block, chosen by its flag byte.
fn pilot_for(block: &[u8]) -> u32 {
    if block.first().is_none_or(|&f| f < 128) {
        PILOT_HEADER_PULSES
    } else {
        PILOT_DATA_PULSES
    }
}

/// Append a pilot+sync+data block (the general turbo form; standard speed is the
/// same with the ROM timings and 8 bits in the last byte).
#[allow(clippy::too_many_arguments)]
fn encode_block(
    pulses: &mut Vec<u32>,
    data: &[u8],
    pilot: u32,
    sync1: u32,
    sync2: u32,
    zero: u32,
    one: u32,
    pilot_pulses: u32,
    last_bits: u32,
    pause_ms: u32,
) {
    for _ in 0..pilot_pulses {
        pulses.push(pilot);
    }
    pulses.push(sync1);
    pulses.push(sync2);
    encode_data_bits(pulses, data, zero, one, last_bits, pause_ms);
}

/// Append just the data bits (two equal pulses per bit, MSB first) and a pause.
fn encode_data_bits(
    pulses: &mut Vec<u32>,
    data: &[u8],
    zero: u32,
    one: u32,
    last_bits: u32,
    pause_ms: u32,
) {
    let n = data.len();
    for (i, &byte) in data.iter().enumerate() {
        let bits = if i + 1 == n { last_bits.clamp(1, 8) } else { 8 };
        for b in (0..bits).rev() {
            let p = if (byte >> b) & 1 == 1 { one } else { zero };
            pulses.push(p);
            pulses.push(p);
        }
    }
    if pause_ms > 0 {
        pulses.push(pause_ms * T_PER_MS);
    }
}

/// `.tzx` → pulses. Handles the common blocks (standard/turbo/pure-tone/pulse/
/// pure-data/pause + simple loops); metadata blocks are skipped by their length.
fn tzx_pulses(data: &[u8]) -> Result<Vec<u32>, SnapshotError> {
    if data.len() < 10 || &data[0..8] != b"ZXTape!\x1a" {
        return Err(SnapshotError::Unsupported("not a .tzx"));
    }
    let mut pulses = Vec::new();
    let mut i = 10; // skip 8-byte magic + 2 version bytes
    let mut loop_start = 0usize;
    let mut loop_count = 0u16;

    while i < data.len() {
        let id = data[i];
        i += 1;
        match id {
            0x10 => {
                // Standard speed data: pause:u16, len:u16, data.
                need(data, i, 4)?;
                let pause = rd16(data, i);
                let len = rd16(data, i + 2) as usize;
                i += 4;
                let block = slice(data, i, len)?;
                i += len;
                encode_block(
                    &mut pulses,
                    block,
                    PILOT,
                    SYNC1,
                    SYNC2,
                    ZERO,
                    ONE,
                    pilot_for(block),
                    8,
                    pause as u32,
                );
            }
            0x11 => {
                // Turbo speed data: full timing header, len:u24, data.
                need(data, i, 18)?;
                let pilot = rd16(data, i) as u32;
                let s1 = rd16(data, i + 2) as u32;
                let s2 = rd16(data, i + 4) as u32;
                let zero = rd16(data, i + 6) as u32;
                let one = rd16(data, i + 8) as u32;
                let pilot_n = rd16(data, i + 10) as u32;
                let last_bits = data[i + 12] as u32;
                let pause = rd16(data, i + 13) as u32;
                let len = rd24(data, i + 15) as usize;
                i += 18;
                let block = slice(data, i, len)?;
                i += len;
                encode_block(
                    &mut pulses,
                    block,
                    pilot,
                    s1,
                    s2,
                    zero,
                    one,
                    pilot_n,
                    last_bits,
                    pause,
                );
            }
            0x12 => {
                // Pure tone: pulse:u16, count:u16.
                need(data, i, 4)?;
                let pulse = rd16(data, i) as u32;
                let count = rd16(data, i + 2);
                i += 4;
                for _ in 0..count {
                    pulses.push(pulse);
                }
            }
            0x13 => {
                // Pulse sequence: count:u8, count × u16.
                need(data, i, 1)?;
                let n = data[i] as usize;
                i += 1;
                need(data, i, 2 * n)?;
                for k in 0..n {
                    pulses.push(rd16(data, i + 2 * k) as u32);
                }
                i += 2 * n;
            }
            0x14 => {
                // Pure data (no pilot/sync): bit0:u16, bit1:u16, last_bits, pause:u16, len:u24.
                need(data, i, 10)?;
                let zero = rd16(data, i) as u32;
                let one = rd16(data, i + 2) as u32;
                let last_bits = data[i + 4] as u32;
                let pause = rd16(data, i + 5) as u32;
                let len = rd24(data, i + 7) as usize;
                i += 10;
                let block = slice(data, i, len)?;
                i += len;
                encode_data_bits(&mut pulses, block, zero, one, last_bits, pause);
            }
            0x20 => {
                // Pause / stop the tape.
                need(data, i, 2)?;
                let pause = rd16(data, i) as u32;
                i += 2;
                if pause > 0 {
                    pulses.push(pause * T_PER_MS);
                }
            }
            0x24 => {
                // Loop start: count:u16.
                need(data, i, 2)?;
                loop_count = rd16(data, i);
                i += 2;
                loop_start = i;
            }
            0x25 => {
                // Loop end.
                if loop_count > 1 {
                    loop_count -= 1;
                    i = loop_start;
                }
            }
            0x21 => {
                need(data, i, 1)?;
                let n = data[i] as usize;
                i += 1 + n;
            } // group start
            0x22 => {}      // group end
            0x23 => i += 2, // jump (ignored)
            0x27 => {}      // return from call seq
            0x2a => i += 4, // stop tape if 48K
            0x2b => i += 5, // set signal level
            0x30 => {
                need(data, i, 1)?;
                let n = data[i] as usize;
                i += 1 + n;
            } // text description
            0x31 => {
                need(data, i, 2)?;
                let n = data[i + 1] as usize;
                i += 2 + n;
            } // message
            0x32 => {
                let n = rd16(data, i) as usize;
                i += 2 + n;
            } // archive info
            0x33 => {
                need(data, i, 1)?;
                let n = data[i] as usize;
                i += 1 + 3 * n;
            } // hardware type
            0x35 => {
                need(data, i, 20)?;
                let n = rd32(data, i + 16) as usize;
                i += 20 + n;
            } // custom info
            0x5a => i += 9, // glue block
            _ => break,     // unknown block: can't know its length, stop here
        }
    }
    Ok(pulses)
}

fn need(d: &[u8], i: usize, n: usize) -> Result<(), SnapshotError> {
    if i + n > d.len() {
        Err(SnapshotError::Truncated)
    } else {
        Ok(())
    }
}

fn slice(d: &[u8], i: usize, len: usize) -> Result<&[u8], SnapshotError> {
    need(d, i, len)?;
    Ok(&d[i..i + len])
}

fn rd16(d: &[u8], i: usize) -> u16 {
    d[i] as u16 | (d[i + 1] as u16) << 8
}
fn rd24(d: &[u8], i: usize) -> u32 {
    d[i] as u32 | (d[i + 1] as u32) << 8 | (d[i + 2] as u32) << 16
}
fn rd32(d: &[u8], i: usize) -> u32 {
    rd24(d, i) | (d[i + 3] as u32) << 24
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap TZX blocks in a minimal "ZXTape!" container.
    fn tzx(blocks: &[u8]) -> Vec<u8> {
        let mut v = b"ZXTape!\x1a".to_vec();
        v.push(1);
        v.push(20);
        v.extend_from_slice(blocks);
        v
    }

    #[test]
    fn tap_block_has_standard_pilot_sync_and_bits() {
        // One data block (flag 0xFF >= 128 → short pilot) holding bytes FF, 00.
        let tap = [0x02, 0x00, 0xFF, 0x00];
        let sig = TapeSignal::from_bytes("tap", &tap).unwrap();
        let p = &sig.pulses;
        assert_eq!(
            p[..PILOT_DATA_PULSES as usize],
            vec![PILOT; PILOT_DATA_PULSES as usize][..]
        );
        let mut i = PILOT_DATA_PULSES as usize;
        assert_eq!((p[i], p[i + 1]), (SYNC1, SYNC2));
        i += 2;
        // 0xFF = eight 1-bits → 16 pulses of ONE.
        assert!(p[i..i + 16].iter().all(|&x| x == ONE));
        i += 16;
        // 0x00 = eight 0-bits → 16 pulses of ZERO.
        assert!(p[i..i + 16].iter().all(|&x| x == ZERO));
        i += 16;
        // Trailing pause (1000 ms).
        assert_eq!(p[i], 1000 * T_PER_MS);
        assert_eq!(i + 1, p.len());
    }

    #[test]
    fn tzx_pure_tone_and_standard_block() {
        // 0x12 pure tone: pulse=100, count=4.
        let sig = TapeSignal::from_bytes("tzx", &tzx(&[0x12, 100, 0, 4, 0])).unwrap();
        assert_eq!(sig.pulses, vec![100, 100, 100, 100]);

        // 0x10 standard data: pause=0, len=1, byte 0xFF (flag>=128 → short pilot).
        let sig = TapeSignal::from_bytes("tzx", &tzx(&[0x10, 0, 0, 1, 0, 0xFF])).unwrap();
        assert_eq!(sig.pulses[0], PILOT);
        assert_eq!(
            sig.pulses[..PILOT_DATA_PULSES as usize]
                .iter()
                .filter(|&&x| x == PILOT)
                .count(),
            PILOT_DATA_PULSES as usize
        );
    }

    #[test]
    fn tzx_skips_metadata_blocks() {
        // A text block (0x30) then a pure tone — the text must be skipped cleanly.
        let mut blocks = vec![0x30, 3, b'h', b'i', b'!'];
        blocks.extend_from_slice(&[0x12, 50, 0, 2, 0]);
        let sig = TapeSignal::from_bytes("tzx", &tzx(&blocks)).unwrap();
        assert_eq!(sig.pulses, vec![50, 50]);
    }

    #[test]
    fn advance_toggles_ear_level_at_each_boundary() {
        let mut sig = TapeSignal::from_bytes("tzx", &tzx(&[0x12, 100, 0, 4, 0])).unwrap();
        sig.play();
        assert!(!sig.level());
        sig.advance(50);
        assert!(!sig.level(), "mid-pulse: no edge yet");
        sig.advance(50);
        assert!(sig.level(), "crossed the first boundary");
        sig.advance(100);
        assert!(!sig.level());
        sig.advance(200); // consume the last two pulses
        assert!(!sig.playing(), "stops at end of stream");
    }

    #[test]
    fn rejects_non_tzx() {
        assert!(matches!(
            TapeSignal::from_bytes("tzx", b"not a tape"),
            Err(SnapshotError::Unsupported(_))
        ));
    }
}

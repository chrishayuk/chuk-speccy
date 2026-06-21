//! Snapshot loaders. `.sna` (dead simple) and `.z80` v1/v2/v3 (versioned,
//! RLE-compressed). See `docs/01-core-emulator-spec.md` §7.

use crate::Spectrum;

#[derive(Debug)]
pub enum SnapshotError {
    /// The data is too short / malformed for the claimed format.
    Truncated,
    /// The format isn't supported yet (e.g. 128K snapshots on a 48K machine).
    Unsupported(&'static str),
}

const HEADER: usize = 27;
const RAM_LEN: usize = 49152; // 0x4000..=0xFFFF

/// Load a 48K `.sna` snapshot: 27-byte register header + 48K RAM dump. PC is not
/// in the header — it's on the stack, so after splatting state we pop it
/// (`RETN`-style) and copy IFF2 into IFF1.
pub fn load_sna(spec: &mut Spectrum, data: &[u8]) -> Result<(), SnapshotError> {
    if data.len() < HEADER + RAM_LEN {
        return Err(SnapshotError::Truncated);
    }
    // RAM contents (0x4000..=0xFFFF).
    spec.write_memory(0x4000, &data[HEADER..HEADER + RAM_LEN]);

    let rd16 = |o: usize| (data[o] as u16) | ((data[o + 1] as u16) << 8);
    {
        let r = &mut spec.cpu.regs;
        r.i = data[0];
        r.l_ = data[1];
        r.h_ = data[2];
        r.e_ = data[3];
        r.d_ = data[4];
        r.c_ = data[5];
        r.b_ = data[6];
        r.f_ = data[7];
        r.a_ = data[8];
        r.l = data[9];
        r.h = data[10];
        r.e = data[11];
        r.d = data[12];
        r.c = data[13];
        r.b = data[14];
        r.iy = rd16(15);
        r.ix = rd16(17);
        r.r = data[20];
        r.f = data[21];
        r.a = data[22];
        r.sp = rd16(23);
    }
    let iff2 = data[19] & 0x04 != 0;
    spec.cpu.iff1 = iff2;
    spec.cpu.iff2 = iff2;
    spec.cpu.im = data[25] & 0x03;
    spec.cpu.halted = false;
    spec.board.ula.border = data[26] & 0x07;

    // Pop PC off the stack (the 48K .sna quirk), then advance SP past it.
    let sp = spec.cpu.regs.sp;
    let lo = spec.board.mem.read(sp) as u16;
    let hi = spec.board.mem.read(sp.wrapping_add(1)) as u16;
    spec.cpu.regs.pc = lo | (hi << 8);
    spec.cpu.regs.sp = sp.wrapping_add(2);
    Ok(())
}

/// Save the current machine state as a 48K `.sna`. PC is pushed onto a *copy* of
/// the stack (the live machine is left untouched), as the format requires.
pub fn save_sna(spec: &Spectrum) -> Vec<u8> {
    let mut out = vec![0u8; HEADER + RAM_LEN];
    let r = &spec.cpu.regs;
    out[0] = r.i;
    out[1] = r.l_;
    out[2] = r.h_;
    out[3] = r.e_;
    out[4] = r.d_;
    out[5] = r.c_;
    out[6] = r.b_;
    out[7] = r.f_;
    out[8] = r.a_;
    out[9] = r.l;
    out[10] = r.h;
    out[11] = r.e;
    out[12] = r.d;
    out[13] = r.c;
    out[14] = r.b;
    out[15] = r.iy as u8;
    out[16] = (r.iy >> 8) as u8;
    out[17] = r.ix as u8;
    out[18] = (r.ix >> 8) as u8;
    out[19] = if spec.cpu.iff2 { 0x04 } else { 0 };
    out[20] = r.r;
    out[21] = r.f;
    out[22] = r.a;
    let sp = r.sp.wrapping_sub(2); // PC will be pushed here
    out[23] = sp as u8;
    out[24] = (sp >> 8) as u8;
    out[25] = spec.cpu.im;
    out[26] = spec.board.ula.border;

    out[HEADER..].copy_from_slice(&spec.read_memory(0x4000, RAM_LEN as u16));
    // Push PC into the RAM image (only the part that lives in RAM).
    let pc = r.pc;
    for (i, byte) in [pc as u8, (pc >> 8) as u8].into_iter().enumerate() {
        let addr = sp.wrapping_add(i as u16);
        if addr >= 0x4000 {
            out[HEADER + (addr as usize - 0x4000)] = byte;
        }
    }
    out
}

/// Load a `.z80` snapshot (v1, v2, v3) for a 48K machine. 128K snapshots are
/// rejected. RAM may be RLE-compressed; v2/v3 store it as pages.
pub fn load_z80(spec: &mut Spectrum, data: &[u8]) -> Result<(), SnapshotError> {
    if data.len() < 30 {
        return Err(SnapshotError::Truncated);
    }
    let rd16 = |o: usize| (data[o] as u16) | ((data[o + 1] as u16) << 8);

    // --- common 30-byte v1 header ---
    let a = data[0];
    let f = data[1];
    let bc = rd16(2);
    let hl = rd16(4);
    let mut pc = rd16(6);
    let sp = rd16(8);
    let i = data[10];
    let r_low = data[11] & 0x7f;
    let byte12 = if data[12] == 0xff { 1 } else { data[12] };
    let r = (r_low) | ((byte12 & 0x01) << 7);
    let border = (byte12 >> 1) & 0x07;
    let v1_compressed = byte12 & 0x20 != 0;
    let de = rd16(13);
    let bc_ = rd16(15);
    let de_ = rd16(17);
    let hl_ = rd16(19);
    let a_ = data[21];
    let f_ = data[22];
    let iy = rd16(23);
    let ix = rd16(25);
    let iff1 = data[27] != 0;
    let iff2 = data[28] != 0;
    let im = data[29] & 0x03;

    // --- locate and decode RAM ---
    if pc != 0 {
        // v1: a single 48K block at offset 30 (optionally RLE-compressed).
        let payload = &data[30..];
        let ram = if v1_compressed {
            decompress_z80(payload, RAM_LEN)
        } else {
            if payload.len() < RAM_LEN {
                return Err(SnapshotError::Truncated);
            }
            payload[..RAM_LEN].to_vec()
        };
        spec.write_memory(0x4000, &ram);
    } else {
        // v2/v3: extended header, then pages.
        let ext_len = rd16(30) as usize;
        let ext_start = 32;
        if data.len() < ext_start + ext_len {
            return Err(SnapshotError::Truncated);
        }
        pc = rd16(ext_start); // real PC lives in the extended header
        let hw = data[ext_start + 2];
        // 48K hardware modes only: 0 (48K), 1 (48K+IF1). Reject 128K (>=3, except some).
        if hw >= 3 {
            return Err(SnapshotError::Unsupported("z80: 128K hardware not supported (48K only)"));
        }
        let mut off = ext_start + ext_len;
        while off + 3 <= data.len() {
            let blk_len = rd16(off) as usize;
            let page = data[off + 2];
            off += 3;
            let (block, consumed) = if blk_len == 0xffff {
                // uncompressed 16K
                if off + 0x4000 > data.len() {
                    break;
                }
                (data[off..off + 0x4000].to_vec(), 0x4000)
            } else {
                if off + blk_len > data.len() {
                    break;
                }
                (decompress_z80(&data[off..off + blk_len], 0x4000), blk_len)
            };
            // 48K page numbering: 4 -> 0x8000, 5 -> 0xC000, 8 -> 0x4000.
            let base = match page {
                4 => 0x8000,
                5 => 0xC000,
                8 => 0x4000,
                _ => {
                    off += consumed;
                    continue;
                }
            };
            spec.write_memory(base, &block);
            off += consumed;
        }
    }

    {
        let reg = &mut spec.cpu.regs;
        reg.a = a;
        reg.f = f;
        reg.set_bc(bc);
        reg.set_de(de);
        reg.set_hl(hl);
        reg.a_ = a_;
        reg.f_ = f_;
        reg.b_ = (bc_ >> 8) as u8;
        reg.c_ = bc_ as u8;
        reg.d_ = (de_ >> 8) as u8;
        reg.e_ = de_ as u8;
        reg.h_ = (hl_ >> 8) as u8;
        reg.l_ = hl_ as u8;
        reg.ix = ix;
        reg.iy = iy;
        reg.sp = sp;
        reg.pc = pc;
        reg.i = i;
        reg.r = r;
    }
    spec.cpu.iff1 = iff1;
    spec.cpu.iff2 = iff2;
    spec.cpu.im = im;
    spec.cpu.halted = false;
    spec.board.ula.border = border;
    Ok(())
}

/// Decode the `.z80` RLE scheme into exactly `expected` bytes (best effort; stops
/// at the end marker or when full). A run is `ED ED <count> <byte>`.
fn decompress_z80(src: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < src.len() && out.len() < expected {
        if i + 3 < src.len() && src[i] == 0xED && src[i + 1] == 0xED {
            let count = src[i + 2] as usize;
            let value = src[i + 3];
            out.extend(std::iter::repeat_n(value, count));
            i += 4;
        } else if i + 3 < src.len()
            && src[i] == 0x00
            && src[i + 1] == 0xED
            && src[i + 2] == 0xED
            && src[i + 3] == 0x00
        {
            break; // v1 end marker
        } else {
            out.push(src[i]);
            i += 1;
        }
    }
    out.resize(expected, 0);
    out
}

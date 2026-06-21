//! `.tap` emitter — wrap compiled machine code in a tiny BASIC autoloader so it
//! boots on a real 48K ROM (or our emulator). The tape holds four standard
//! blocks: a BASIC program header + data, then a CODE header + data. The BASIC
//! line is:
//!
//! ```text
//! 10 CLEAR <org-1>: LOAD "" CODE: RANDOMIZE USR <entry>
//! ```
//!
//! Loading the tape auto-runs line 10, which protects memory above `org`, loads
//! the CODE block there, and jumps to `entry`.

/// Build a `.tap` that loads `code` at `org` and runs it from `entry`.
pub fn to_tap(code: &[u8], org: u16, entry: u16, name: &str) -> Vec<u8> {
    let basic = basic_loader(org, entry);
    let blen = basic.len() as u16;
    let mut tap = Vec::new();
    // BASIC program: header (autostart line 10, no variables) + data.
    tap.extend(tap_block(0x00, &header(0, name, blen, 10, blen)));
    tap.extend(tap_block(0xFF, &basic));
    // CODE: header (load address `org`) + the bytes.
    tap.extend(tap_block(0x00, &header(3, name, code.len() as u16, org, 0x8000)));
    tap.extend(tap_block(0xFF, code));
    tap
}

/// A 17-byte tape header. `p1`/`p2` are the type-specific parameters
/// (BASIC: autostart line / variable offset; CODE: load address / unused).
fn header(kind: u8, name: &str, len: u16, p1: u16, p2: u16) -> [u8; 17] {
    let mut h = [b' '; 17];
    h[0] = kind;
    let nm = name.as_bytes();
    for (i, slot) in h[1..11].iter_mut().enumerate() {
        *slot = nm.get(i).copied().unwrap_or(b' ');
    }
    h[11..13].copy_from_slice(&len.to_le_bytes());
    h[13..15].copy_from_slice(&p1.to_le_bytes());
    h[15..17].copy_from_slice(&p2.to_le_bytes());
    h
}

/// The tokenised autoloader program (one line, number 10).
fn basic_loader(org: u16, entry: u16) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0xFD); // CLEAR
    push_num(&mut body, org - 1);
    body.push(0x3A); // :
    body.push(0xEF); // LOAD
    body.push(0x22); // "
    body.push(0x22); // "
    body.push(0xAF); // CODE
    body.push(0x3A); // :
    body.push(0xF9); // RANDOMIZE
    body.push(0xC0); // USR
    push_num(&mut body, entry);
    body.push(0x0D); // ENTER

    let mut prog = vec![0x00, 0x0A]; // line number 10 (big-endian)
    prog.extend_from_slice(&(body.len() as u16).to_le_bytes());
    prog.extend_from_slice(&body);
    prog
}

/// A BASIC numeric literal: ASCII digits then the hidden `0x0E` + 5-byte integer
/// form (`[exp=0, sign=0, LSB, MSB, 0]`).
fn push_num(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(v.to_string().as_bytes());
    out.push(0x0E);
    out.extend_from_slice(&[0x00, 0x00, v as u8, (v >> 8) as u8, 0x00]);
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

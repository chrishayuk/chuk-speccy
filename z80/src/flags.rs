//! Flag bit positions and the precomputed flag tables.
//!
//! Build a 256-entry `SZ53P` table (sign, zero, the two undocumented bits 5/3
//! copied from the result, and parity) and a separate `SZ53` for non-parity ops.
//! ALU carry/half-carry are computed inline at the opcode site.
//! See `docs/01-core-emulator-spec.md` §4.3.

// Flag bit masks (Z80 F register).
pub const CF: u8 = 1 << 0; // carry
pub const NF: u8 = 1 << 1; // add/subtract
pub const PF: u8 = 1 << 2; // parity/overflow
pub const XF: u8 = 1 << 3; // undocumented (bit 3 of result)
pub const HF: u8 = 1 << 4; // half-carry
pub const YF: u8 = 1 << 5; // undocumented (bit 5 of result)
pub const ZF: u8 = 1 << 6; // zero
pub const SF: u8 = 1 << 7; // sign

/// `S`, `Z`, `Y`, `X` for a given byte (no parity).
const fn sz53(n: u8) -> u8 {
    (n & (SF | YF | XF)) | if n == 0 { ZF } else { 0 }
}

const fn parity(mut n: u8) -> bool {
    let mut bits = 0u8;
    let mut i = 0;
    while i < 8 {
        bits += n & 1;
        n >>= 1;
        i += 1;
    }
    bits & 1 == 0 // PF set when parity is even
}

const fn build_sz53() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = sz53(i as u8);
        i += 1;
    }
    t
}

const fn build_sz53p() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut f = sz53(i as u8);
        if parity(i as u8) {
            f |= PF;
        }
        t[i] = f;
        i += 1;
    }
    t
}

/// SZ53 for non-parity results (e.g. ALU add/sub set P=overflow separately).
pub const SZ53: [u8; 256] = build_sz53();
/// SZ53 + parity, for logic ops and others that use P=parity.
pub const SZ53P: [u8; 256] = build_sz53p();

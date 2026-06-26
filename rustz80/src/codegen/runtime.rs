//! The appended mul/div micro-runtime (Spectrum target) + the Cell80 `ED FE` trap ids.
use super::asm::Asm;

/// `__mul16`: HL = HL * DE (low 16). Shift-add, **multiplier-terminated**: loops once
/// per bit up to the multiplier's (DE's) top set bit, then returns — so small operands
/// finish in a few iterations instead of a fixed 16. Clobbers AF/BC/DE.
pub(super) const MUL16: &[u8] = &[
    0x44, 0x4D, // ld b,h ; ld c,l   (BC = multiplicand)
    0x21, 0x00, 0x00, // ld hl,0     (product)
    // loop:
    0x7B, 0xB2, 0xC8, // ld a,e ; or d ; ret z   (no multiplier bits left → done)
    0xCB, 0x3A, 0xCB, 0x1B, // srl d ; rr e       (DE >>= 1, low bit -> CF)
    0x30, 0x01, // jr nc,+1
    0x09, // add hl,bc                            (product += multiplicand)
    // skip:
    0xCB, 0x21, 0xCB, 0x10, // sla c ; rl b       (BC <<= 1)
    0x18, 0xF0, // jr loop  (-16)
];

/// `__divmod16`: HL/DE -> HL=quotient, DE=remainder (divisor < 0x8000).
/// Fast path: `dividend < divisor` → quotient 0, remainder = dividend (returns at once).
/// Else restoring division. Clobbers AF/BC.
pub(super) const DIVMOD16: &[u8] = &[
    // Fast path: if HL (dividend) < DE (divisor), q=0, r=dividend.
    0x7C, 0xBA, // ld a,h ; cp d
    0x38, 0x06, // jr c, less        (H < D → HL < DE)
    0x20, 0x09, // jr nz, big        (H > D → HL >= DE)
    0x7D, 0xBB, // ld a,l ; cp e     (H == D: compare low)
    0x30, 0x05, // jr nc, big        (L >= E → HL >= DE)
    // less: quotient 0, remainder = dividend.
    0xEB, // ex de,hl                (DE = dividend = remainder, HL = divisor)
    0x21, 0x00, 0x00, // ld hl,0     (quotient)
    0xC9, // ret
    // big: restoring division.
    0x44, 0x4D, // ld b,h ; ld c,l   (BC = dividend)
    0x21, 0x00, 0x00, // ld hl,0     (remainder)
    0x3E, 0x10, // ld a,16
    0xCB, 0x21, 0xCB, 0x10, // sla c ; rl b   (BC <<= 1, MSB -> CF)
    0xED, 0x6A, // adc hl,hl   (rem = rem*2 + bit)
    0xED, 0x52, // sbc hl,de   (rem -= divisor)
    0x30, 0x03, // jr nc,+3 -> set
    0x19, // add hl,de   (restore)
    0x18, 0x01, // jr +1 -> cont
    0x0C, // set: inc c   (quotient bit)
    0x3D, 0x20, 0xEF, // cont: dec a ; jr nz
    0xEB, // ex de,hl    (DE = remainder)
    0x60, 0x69, // ld h,b ; ld l,c   (HL = quotient)
    0xC9, // ret
];

/// Host-trap ids (match `spectrum::host::math_traps`): `HL = BC * DE`, and `HL = BC / DE`
/// with `DE = BC % DE`.
pub(super) const TRAP_MUL16: u8 = 0x10;

pub(super) const TRAP_DIVMOD16: u8 = 0x11;

/// Emit a host trap: `LD A, id ; ED FE` (the reserved `TRAP_OP`).
pub(super) fn gen_trap(a: &mut Asm, id: u8) {
    a.byte(0x3E); // LD A, id
    a.byte(id);
    a.byte(0xED); // ED FE  (host trap)
    a.byte(0xFE);
}

pub(super) const TRAP_FILL16: u8 = 0x20; // fill `BC` slots (2-byte words) at `HL` with `DE`

pub(super) const TRAP_HALT: u8 = 0x30; // stop the run with status code `HL`

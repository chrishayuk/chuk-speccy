//! Bit-exact full-machine state: `serialize_full` / `deserialize_full`.
//!
//! Unlike `.sna`/`.z80` (which drop or approximate the subtle bits), this captures
//! **everything that affects future execution** — every CPU register including
//! MEMPTR/`wz`, the `q`/`q_prev` latch, `iff1`/`iff2`, `im`, `halted`,
//! `ei_pending`; the full 48K RAM; the ULA frame phase (`tstate`/`frame`), border,
//! beeper level + audio carry; and the keyboard matrix + EAR bit. The ROM and the
//! precomputed contention table are constants (rebuilt by `new_48k`), and the
//! host-trap dispatcher / recording buffers are runtime concerns, so none are
//! stored.
//!
//! This is the precondition for using the machine as a deterministic RL/agent
//! substrate: reset is a non-source-of-variance, so a snapshot is an exact
//! timeline branch (and the snapshot tree can stand in for MCTS rollouts).

use crate::Spectrum;

const MAGIC: &[u8; 4] = b"CSP1";
const RAM_BYTES: usize = 0xC000; // 48K, $4000..$10000
/// magic + CPU(35) + ULA(25) + keyboard(9) + RAM.
const TOTAL: usize = 4 + 35 + 25 + 9 + RAM_BYTES;

/// A little-endian read cursor over a serialized blob (lengths pre-validated).
pub(crate) struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cur { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        s
    }
    pub(crate) fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    pub(crate) fn bool(&mut self) -> bool {
        self.u8() != 0
    }
    pub(crate) fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    pub(crate) fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    pub(crate) fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.take(8).try_into().unwrap())
    }
}

impl Spectrum {
    /// Serialize the complete machine state to a portable blob (see module docs).
    pub fn serialize_full(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(TOTAL);
        o.extend_from_slice(MAGIC);

        let r = &self.cpu.regs;
        o.extend_from_slice(&[
            r.a, r.f, r.b, r.c, r.d, r.e, r.h, r.l, // main set
            r.a_, r.f_, r.b_, r.c_, r.d_, r.e_, r.h_, r.l_, // shadow set
        ]);
        o.extend_from_slice(&r.ix.to_le_bytes());
        o.extend_from_slice(&r.iy.to_le_bytes());
        o.extend_from_slice(&r.sp.to_le_bytes());
        o.extend_from_slice(&r.pc.to_le_bytes());
        o.push(r.i);
        o.push(r.r);
        o.extend_from_slice(&r.wz.to_le_bytes());
        o.push(self.cpu.iff1 as u8);
        o.push(self.cpu.iff2 as u8);
        o.push(self.cpu.im);
        o.push(self.cpu.halted as u8);
        o.push(self.cpu.q);
        o.push(self.cpu.q_prev);
        o.push(self.cpu.ei_pending as u8);

        self.board.ula.save(&mut o);
        self.board.kb.save(&mut o);
        o.extend_from_slice(&self.read_memory(0x4000, RAM_BYTES as u16));
        debug_assert_eq!(o.len(), TOTAL);
        o
    }

    /// Restore a [`serialize_full`](Self::serialize_full) blob into this machine
    /// (ROM and the host-trap dispatcher are left untouched). Errors on a blob
    /// whose magic or length doesn't match.
    pub fn deserialize_full(&mut self, blob: &[u8]) -> Result<(), String> {
        if blob.len() != TOTAL || &blob[..4] != MAGIC {
            return Err(format!("not a full-state blob (len {}, want {TOTAL})", blob.len()));
        }
        let mut c = Cur::new(blob);
        let _ = c.take(4); // magic

        let r = &mut self.cpu.regs;
        r.a = c.u8();
        r.f = c.u8();
        r.b = c.u8();
        r.c = c.u8();
        r.d = c.u8();
        r.e = c.u8();
        r.h = c.u8();
        r.l = c.u8();
        r.a_ = c.u8();
        r.f_ = c.u8();
        r.b_ = c.u8();
        r.c_ = c.u8();
        r.d_ = c.u8();
        r.e_ = c.u8();
        r.h_ = c.u8();
        r.l_ = c.u8();
        r.ix = c.u16();
        r.iy = c.u16();
        r.sp = c.u16();
        r.pc = c.u16();
        r.i = c.u8();
        r.r = c.u8();
        r.wz = c.u16();
        self.cpu.iff1 = c.bool();
        self.cpu.iff2 = c.bool();
        self.cpu.im = c.u8();
        self.cpu.halted = c.bool();
        self.cpu.q = c.u8();
        self.cpu.q_prev = c.u8();
        self.cpu.ei_pending = c.bool();

        self.board.ula.load(&mut c);
        self.board.kb.load(&mut c);
        let ram = c.take(RAM_BYTES).to_vec();
        self.write_memory(0x4000, &ram);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Spectrum;

    fn rom() -> Option<Vec<u8>> {
        std::env::var("SPECTRUM_ROM").ok().and_then(|p| std::fs::read(p).ok())
    }

    /// Reset must be a non-source-of-variance: a restored machine evolves
    /// **bit-for-bit identically** to the original. Run from real ROM state.
    ///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p spectrum -- --ignored serialize_full
    #[test]
    #[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
    fn serialize_full_is_bit_exact() {
        let rom = rom().expect("SPECTRUM_ROM");
        let mut a = Spectrum::new_48k(&rom);
        for _ in 0..200 {
            a.run_frame(); // boot to the prompt
        }
        a.type_text("PRINT 6*7\n"); // dirty the keyboard / RAM / ULA phase
        for _ in 0..20 {
            a.run_frame();
        }

        let blob = a.serialize_full();
        let mut b = Spectrum::new_48k(&rom);
        b.deserialize_full(&blob).unwrap();

        // Exact round-trip.
        assert_eq!(b.serialize_full(), blob, "restore should reproduce the blob");

        // And both evolve identically — full state equal every frame. If any
        // execution-affecting field were dropped, the two would diverge.
        for f in 0..300 {
            a.run_frame();
            b.run_frame();
            assert_eq!(a.serialize_full(), b.serialize_full(), "state diverged at frame {f}");
        }
    }

    #[test]
    fn deserialize_rejects_garbage() {
        let mut s = Spectrum::new_48k(&[0u8; 0x4000]);
        assert!(s.deserialize_full(b"nope").is_err());
        assert!(s.deserialize_full(&[0u8; 10]).is_err());
    }
}

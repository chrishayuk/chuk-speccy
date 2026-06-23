//! Snake, written in the rustz80 dialect, compiled to Z80, and run on the CPU —
//! the end-to-end demonstration (spec 07). The same algorithm is replicated in
//! plain Rust here as the oracle: after N steps, the compiled game's **final
//! state checksum** and its **screen bitmap** must both match the Rust replica
//! byte-for-byte.
//!
//! Exercises essentially the whole dialect at once: multi-function programs + the
//! calling convention, `u16`/`u8`, arrays (`bx`/`by` body), `if`/`while`, `match`
//! (steering), `*`/`/`/`%` (the screen-address math), bitwise `|`/`^`, and the
//! `poke`/`peek` raw-memory intrinsics drawing into real screen RAM.

// The reference deliberately mirrors the dialect's long-form (`x = x + 1`, no
// `+=`) so the two read identically when audited side by side.
#![allow(clippy::assign_op_pattern)]

/// A flat 64K RAM bus.
struct Ram {
    mem: Vec<u8>,
}

impl z80::Bus for Ram {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
    }
    fn input(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _: u16, _: u8) {}
    fn contend(&mut self, _: u16, _: u32) {}
    fn tick(&mut self, _: u32) {}
}

/// The game, in the dialect. `poke`/`peek` are prelude intrinsics (the compiler
/// supplies them); everything else is ordinary dialect code.
const SNAKE: &str = r#"
    fn mask_of(x: u16) -> u16 {
        let masks = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
        masks[(x % 8u16) as usize] as u16
    }
    fn addr_of(x: u16, y: u16) -> u16 {
        16384u16
            + (y / 64u16) * 2048u16
            + (y % 8u16) * 256u16
            + ((y / 8u16) % 8u16) * 32u16
            + x / 8u16
    }
    fn set_px(x: u16, y: u16) {
        let a = addr_of(x, y);
        let m = mask_of(x);
        poke(a, peek(a) | m);
    }
    fn clr_px(x: u16, y: u16) {
        let a = addr_of(x, y);
        let m = mask_of(x);
        poke(a, peek(a) ^ m);
    }
    fn run(steps: u16) -> u16 {
        let mut bx = [0u16; 6];
        let mut by = [0u16; 6];
        let mut i = 0u16;
        while i < 6u16 {
            bx[i as usize] = 5u16 - i;
            by[i as usize] = 12u16;
            set_px(bx[i as usize] * 8u16, by[i as usize] * 8u16);
            i = i + 1u16;
        }
        let mut dir = 0u16;
        let mut s = 0u16;
        while s < steps {
            if (s % 5u16) == 4u16 {
                dir = (dir + 1u16) % 4u16;
            }
            let mut nx = bx[0];
            let mut ny = by[0];
            match dir {
                0u16 => nx = (nx + 1u16) % 32u16,
                1u16 => ny = (ny + 1u16) % 24u16,
                2u16 => nx = (nx + 31u16) % 32u16,
                _ => ny = (ny + 23u16) % 24u16,
            }
            clr_px(bx[5] * 8u16, by[5] * 8u16);
            let mut j = 5u16;
            while j > 0u16 {
                bx[j as usize] = bx[(j - 1u16) as usize];
                by[j as usize] = by[(j - 1u16) as usize];
                j = j - 1u16;
            }
            bx[0] = nx;
            by[0] = ny;
            set_px(nx * 8u16, ny * 8u16);
            s = s + 1u16;
        }
        let mut sum = 0u16;
        let mut k = 0u16;
        while k < 6u16 {
            sum = sum + (bx[k as usize] + 1u16) * (by[k as usize] + 1u16) * (k + 1u16);
            k = k + 1u16;
        }
        sum
    }
"#;

fn addr_of(x: u16, y: u16) -> usize {
    (0x4000 + ((y & 0xC0) << 5) + ((y & 0x07) << 8) + ((y & 0x38) << 2) + (x >> 3)) as usize
}
fn mask_of(x: u16) -> u8 {
    0x80u8 >> (x % 8)
}

/// The oracle: the identical algorithm in plain Rust, returning (checksum, screen).
#[allow(unused_assignments)]
fn snake_reference(steps: u16) -> (u16, Vec<u8>) {
    let mut scr = vec![0u8; 0x1_0000];
    let mut bx = [0u16; 6];
    let mut by = [0u16; 6];
    for i in 0..6u16 {
        bx[i as usize] = 5 - i;
        by[i as usize] = 12;
        scr[addr_of(bx[i as usize] * 8, by[i as usize] * 8)] |= mask_of(bx[i as usize] * 8);
    }
    let mut dir = 0u16;
    let mut s = 0u16;
    while s < steps {
        if s % 5 == 4 {
            dir = (dir + 1) % 4;
        }
        let mut nx = bx[0];
        let mut ny = by[0];
        match dir {
            0 => nx = (nx + 1) % 32,
            1 => ny = (ny + 1) % 24,
            2 => nx = (nx + 31) % 32,
            _ => ny = (ny + 23) % 24,
        }
        scr[addr_of(bx[5] * 8, by[5] * 8)] ^= mask_of(bx[5] * 8);
        let mut j = 5u16;
        while j > 0 {
            bx[j as usize] = bx[(j - 1) as usize];
            by[j as usize] = by[(j - 1) as usize];
            j -= 1;
        }
        bx[0] = nx;
        by[0] = ny;
        scr[addr_of(nx * 8, ny * 8)] |= mask_of(nx * 8);
        s += 1;
    }
    let mut sum = 0u16;
    for k in 0..6u16 {
        sum = sum + (bx[k as usize] + 1) * (by[k as usize] + 1) * (k + 1);
    }
    (sum, scr)
}

/// Compile + run the dialect Snake for `steps` (passed in `HL`), returning the
/// final checksum (`HL`) and the 64K memory.
fn run_snake(steps: u16) -> (u16, Vec<u8>) {
    let prog = rustz80::compile_program(SNAKE).expect("compile");
    let run = prog.symbols["run"];
    let mut bus = Ram {
        mem: vec![0u8; 0x1_0000],
    };
    // trampoline @ 0x7000:  LD HL,steps ; CALL run ; HALT
    bus.mem[0x7000] = 0x21;
    bus.mem[0x7001] = steps as u8;
    bus.mem[0x7002] = (steps >> 8) as u8;
    bus.mem[0x7003] = 0xCD;
    bus.mem[0x7004] = run as u8;
    bus.mem[0x7005] = (run >> 8) as u8;
    bus.mem[0x7006] = 0x76;
    let org = rustz80::ORG as usize;
    bus.mem[org..org + prog.code.len()].copy_from_slice(&prog.code);

    let mut cpu = z80::Cpu::new();
    cpu.reset();
    cpu.regs.pc = 0x7000;
    cpu.regs.sp = 0xFFF0;
    for _ in 0..20_000_000 {
        if cpu.halted {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted, "snake did not finish");
    (cpu.regs.hl(), bus.mem)
}

#[test]
fn snake_matches_reference() {
    for steps in [0u16, 1, 5, 12, 30, 64] {
        let (got_sum, got_mem) = run_snake(steps);
        let (want_sum, want_scr) = snake_reference(steps);
        assert_eq!(got_sum, want_sum, "checksum mismatch at {steps} steps");
        assert_eq!(
            &got_mem[0x4000..0x5800],
            &want_scr[0x4000..0x5800],
            "screen bitmap mismatch at {steps} steps"
        );
    }
}

#[test]
fn snake_actually_draws() {
    // After warm-up the body is 6 distinct cells → exactly 6 lit pixels.
    let (_sum, mem) = run_snake(30);
    let lit: u32 = mem[0x4000..0x5800].iter().map(|b| b.count_ones()).sum();
    assert_eq!(lit, 6, "expected the 6-cell snake body lit, got {lit}");
}

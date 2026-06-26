//! Property/fuzz tests for the cell's headline guarantees — determinism and reset
//! completeness — stated adversarially rather than as a few hand-written samples
//! (behind `--features cell`). Seeded + reproducible (a fixed corpus, no external deps).
#![cfg(feature = "cell")]

use rustz80::cell::{CellConfig, CellPool, CellProgram, Halt, Runner, DEFAULT_CYCLES};

/// A tiny deterministic xorshift PRNG — so the corpus is reproducible (and `cargo test`
/// stays free of `rand`).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Generate a random **straight-line** arithmetic body over `a`, `b`, and constants — the
/// fast-path-eligible shape. Cell arithmetic wraps (it's raw Z80), and `/0`/`%0` are defined
/// (`0xFFFF`), so every generated program is valid and total; we test self-consistency, not
/// equivalence to rustc (that's `diff.rs`).
fn gen_expr(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 || rng.below(3) == 0 {
        return match rng.below(3) {
            0 => "a".into(),
            1 => "b".into(),
            _ => format!("{}u16", rng.below(1000)),
        };
    }
    let l = gen_expr(rng, depth - 1);
    let r = gen_expr(rng, depth - 1);
    match rng.below(8) {
        0 => format!("({l}).wrapping_add({r})"),
        1 => format!("({l}).wrapping_sub({r})"),
        2 => format!("({l}).wrapping_mul({r})"),
        3 => format!("(({l}) / (({r}) | 1u16))"),
        4 => format!("(({l}) % (({r}) | 1u16))"),
        5 => format!("(({l}) & ({r}))"),
        6 => format!("(({l}) | ({r}))"),
        _ => format!("(({l}) ^ ({r}))"),
    }
}

fn gen_program(rng: &mut Rng) -> String {
    format!("fn run(a: u16, b: u16) -> u16 {{ {} }}", gen_expr(rng, 4))
}

/// `(result, cycles, halt, touched)` — the full deterministic fingerprint of a run.
type Snapshot = (u16, u64, Halt, Vec<(u16, u16)>);
fn snapshot(r: &mut Runner, args: &[u16]) -> Snapshot {
    let rep = r.run(None, args, DEFAULT_CYCLES).unwrap();
    (rep.result, rep.cycles, rep.halt, rep.touched)
}

#[test]
fn determinism_fuzz() {
    // For random programs × random inputs, the fingerprint `(result, cycles, halt, touched)`
    // must be bit-identical across: (a) re-run on the same Runner, (b) a fresh Runner, (c) a
    // Runner from an image round-tripped through to_bytes/from_bytes; and the fast executor
    // (run_fast / run_many_fast) must agree with the authentic interpreter on result/cycles/halt.
    let inputs: [[u16; 2]; 7] = [
        [0, 0],
        [1, 1],
        [7, 3],
        [0xFFFF, 1],
        [0x8000, 0x8000],
        [40000, 9999],
        [255, 256],
    ];
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let src = gen_program(&mut rng);
        let prog = CellProgram::compile(&src).unwrap();
        let image = prog.to_bytes();
        let reloaded = CellProgram::from_bytes(&image).unwrap();

        let mut r1 = Runner::new(&prog);
        for inp in &inputs {
            let base = snapshot(&mut r1, inp);
            // (a) re-run on the same warm Runner.
            assert_eq!(
                snapshot(&mut r1, inp),
                base,
                "rerun diverged\n{src}\n{inp:?}"
            );
            // (b) a fresh Runner.
            assert_eq!(
                snapshot(&mut Runner::new(&prog), inp),
                base,
                "fresh runner diverged\n{src}\n{inp:?}"
            );
            // (c) a Runner from a round-tripped image.
            assert_eq!(
                snapshot(&mut Runner::new(&reloaded), inp),
                base,
                "image round-trip diverged\n{src}\n{inp:?}"
            );
            // (d) fast executor (single + batch) vs the authentic interpreter.
            let f = r1.run_fast(None, inp, DEFAULT_CYCLES).unwrap();
            assert_eq!(
                (f.result, f.cycles, f.halt),
                (base.0, base.1, base.2),
                "run_fast diverged\n{src}\n{inp:?}"
            );
            let many = r1.run_many_fast(None, &[inp], DEFAULT_CYCLES).unwrap();
            assert_eq!(
                (many[0].result, many[0].cycles, many[0].halt),
                (base.0, base.1, base.2),
                "run_many_fast diverged\n{src}\n{inp:?}"
            );
        }
    }
}

#[test]
fn reset_completeness_across_programs() {
    // The Runner resets only the bytes a run *wrote*. Prove that's complete across **all**
    // 64K when a pooled bus is reused by a *different* program: a writer scribbles three
    // high addresses, then a probe (on the recycled bus) reads them back — it must see the
    // same clean zeros as on a fresh bus. A reset leak would surface here as a non-zero sum.
    let writer = CellProgram::compile_with_config(
        "fn run() -> u16 { poke(0xC000u16, 0xABu8); poke(0xD000u16, 0xCDu8); poke(0xE000u16, 0xEFu8); 0u16 }",
        CellConfig::permissive(),
    )
    .unwrap();
    let probe = CellProgram::compile_with_config(
        "fn run() -> u16 { peek(0xC000u16) as u16 + peek(0xD000u16) as u16 + peek(0xE000u16) as u16 }",
        CellConfig::permissive(),
    )
    .unwrap();

    // Fresh-bus baseline: nothing was ever written there → 0.
    let fresh = Runner::new(&probe)
        .run(None, &[], DEFAULT_CYCLES)
        .unwrap()
        .result;
    assert_eq!(fresh, 0);

    // Reuse the *same* bus across many writer→probe alternations.
    let mut pool = CellPool::new();
    for _ in 0..8 {
        let mut w = pool.acquire(&writer);
        assert_eq!(w.run(None, &[], DEFAULT_CYCLES).unwrap().result, 0);
        pool.release(w);

        let mut p = pool.acquire(&probe); // recycles the bus the writer just used
        let reused = p.run(None, &[], DEFAULT_CYCLES).unwrap().result;
        pool.release(p);
        assert_eq!(
            reused, fresh,
            "reset leaked the writer's high-memory writes into the probe"
        );
    }
}

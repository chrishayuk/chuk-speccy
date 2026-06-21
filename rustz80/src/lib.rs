//! `rustz80` — a restricted Rust → Z80 compiler ([spec 07](../docs/07-rust-z80-compiler-spec.md)).
//!
//! **Stage 0 (proof of life):** parse a Rust `fn` with [`syn`], lower a bounded
//! subset to a tiny typed IR, and emit Z80 machine code — `u16` locals, `+`/`-`,
//! `if/else`, `while`, and comparison conditions. The accepted subset is *also
//! real Rust*, so the same source runs under rustc (host) and compiles here
//! (pure), and the two are checked against each other by differential testing on
//! the emulator (see `tests/diff.rs`).
//!
//! Not an LLVM backend, no real `core`: codegen uses `HL` as the accumulator,
//! `DE` as secondary, and a fixed RAM scratch region as the "register file".

mod codegen;
mod ir;
mod lower;

pub use ir::Func;

/// Where compiled code is laid out (absolute jump targets are resolved against it).
pub const ORG: u16 = 0x8000;

/// Compile a single Rust `fn` (source string) to Z80 machine code laid out at
/// [`ORG`], returning the function's `u16` result in `HL` and `RET`ting.
pub fn compile_fn(src: &str) -> Result<Vec<u8>, String> {
    let item: syn::ItemFn = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let func = lower::lower(&item)?;
    Ok(codegen::codegen(&func, ORG))
}

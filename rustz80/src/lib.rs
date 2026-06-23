//! `rustz80` — a **generic** restricted-Rust → Z80 compiler
//! ([spec 07](../docs/07-rust-z80-compiler-spec.md)).
//!
//! Parse a Rust source with [`syn`], lower a bounded subset to a tiny typed IR, and
//! emit Z80 machine code — `u16`/`u8`, arithmetic, `if`/`while`, `for` ranges,
//! `loop`/`break`/`continue`, early `return`, comparisons, arrays, `struct`/`enum`,
//! functions, methods, and `poke`/`peek`/`inport` intrinsics. The accepted subset is
//! *also real Rust*, so the same source runs under rustc and compiles here, checked
//! against each other by differential testing on the emulator (`tests/diff.rs`).
//!
//! This crate knows nothing about "games" or any particular SDK. Method calls on a
//! *handle* parameter route to free prelude functions via a caller-supplied
//! [`PreludeConfig`]; the game/SDK layer lives above (`chuk-speccy-sdk`).
//!
//! Not an LLVM backend, no real `core`: codegen uses `HL` as the accumulator, `DE`
//! as secondary, and a fixed RAM scratch region as the "register file".

mod codegen;
mod ir;
mod lower;
mod tap;

pub use codegen::codegen_loop;
pub use ir::Func;
pub use lower::{lower_program, PreludeConfig};
pub use tap::to_tap;

use std::collections::HashMap;

/// Where compiled code is laid out (absolute jump targets are resolved against it).
pub const ORG: u16 = 0x8000;

/// A compiled program: the machine code (loaded at [`ORG`]) and the absolute
/// address of each function by name.
pub struct Program {
    pub code: Vec<u8>,
    pub symbols: HashMap<String, u16>,
}

/// Compile a multi-`fn` program. Functions are laid out in source order from
/// [`ORG`]; calls resolve by name; the mul/div micro-runtime is appended if used.
pub fn compile_program(src: &str) -> Result<Program, String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let funcs = lower_program(&file, &PreludeConfig::default())?;
    let (code, symbols) = codegen::codegen_program(&funcs, ORG, None);
    Ok(Program { code, symbols })
}

/// Compile a program and wrap it as a bootable `.tap` that runs from `entry`
/// (a function name, default `"main"`). The autoloader `CLEAR`s below [`ORG`],
/// `LOAD`s the code there, and `RANDOMIZE USR`s the entry.
pub fn compile_to_tap(src: &str, entry: &str, name: &str) -> Result<Vec<u8>, String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let funcs = lower_program(&file, &PreludeConfig::default())?;
    if !funcs.iter().any(|(n, _)| n == entry) {
        return Err(format!("no `{entry}` function"));
    }
    // Emit a DI/EI trampoline at ORG and boot into it (`USR ORG`).
    let (code, _) = codegen::codegen_program(&funcs, ORG, Some(entry));
    Ok(to_tap(&code, ORG, ORG, name))
}

/// Compile a single Rust `fn` to Z80 machine code with its entry at [`ORG`]
/// (result in `HL`, then `RET`). Convenience over [`compile_program`].
pub fn compile_fn(src: &str) -> Result<Vec<u8>, String> {
    let item: syn::ItemFn = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let name = item.sig.ident.to_string();
    let func = lower::lower(&item)?;
    let (code, _) = codegen::codegen_program(&[(name, func)], ORG, None);
    Ok(code)
}

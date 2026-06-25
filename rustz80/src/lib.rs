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

#[cfg(feature = "cell")]
pub mod cell;
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

/// One entry in a [`size_report`]: a named function (or appended runtime routine, or
/// a monomorphized instance) and the byte span it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnSize {
    pub name: String,
    pub addr: u16,
    pub size: u16,
    /// A monomorphized generic instance (its mangled name carries a `$`).
    pub instance: bool,
}

impl Program {
    /// Per-symbol code sizes, in layout order. Each function's size is the gap to the
    /// next symbol's address (the last reaches the end of `code`) — so it accounts for
    /// every monomorphized instance and the appended `__mul16`/`__divmod16` runtime.
    /// Lets you *see* what generics/tuples cost in bytes ([`FnSize::instance`] flags
    /// the instances). Sizes sum to `code.len()` for a [`compile_program`] image.
    pub fn size_report(&self) -> Vec<FnSize> {
        let mut syms: Vec<(&String, u16)> = self.symbols.iter().map(|(n, &a)| (n, a)).collect();
        syms.sort_by_key(|&(_, a)| a);
        let end = ORG.wrapping_add(self.code.len() as u16);
        syms.iter()
            .enumerate()
            .map(|(i, &(name, addr))| {
                let next = syms.get(i + 1).map(|&(_, a)| a).unwrap_or(end);
                FnSize {
                    name: name.clone(),
                    addr,
                    size: next.wrapping_sub(addr),
                    instance: name.contains('$'),
                }
            })
            .collect()
    }
}

/// Compile a multi-`fn` program. Functions are laid out in source order from
/// [`ORG`]; calls resolve by name; the mul/div micro-runtime is appended if used.
pub fn compile_program(src: &str) -> Result<Program, String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let funcs = lower_program(&file, &PreludeConfig::default())?;
    let (code, symbols) = codegen::codegen_program(&funcs, ORG, None);
    Ok(Program { code, symbols })
}

/// One field of a [`struct_layout`]: its name, slot offset, and slot count (1 for a
/// scalar, `N` for `[T; N]` / a tuple, `N × sizeof(elem)` for `[Cell; N]`). Each slot is
/// a 2-byte `u16` cell, so a field's byte address (relative to a struct base) is
/// `offset * 2`. This is the typed-state ABI: the layout a caller writes inputs into and
/// reads outputs out of (`rustz80-cell`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    /// Slot offset from the struct's base.
    pub offset: u16,
    /// Slot count (each slot is one 2-byte `u16`).
    pub slots: u16,
}

/// The field layout of a (non-generic) named struct in `src` — `(name, slot offset, slot
/// count)` in declaration order. Field byte addresses are `base + offset * 2`. Used to
/// place typed inputs / read typed state for a cell whose state lives at a known base.
pub fn struct_layout(src: &str, name: &str) -> Result<Vec<FieldLayout>, String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let structs = lower::layout::collect_structs(&file)?;
    let fields = structs
        .get(name)
        .ok_or_else(|| format!("no struct `{name}`"))?;
    let mut out = Vec::with_capacity(fields.len());
    let mut offset = 0u16;
    for f in fields {
        out.push(FieldLayout {
            name: f.name.clone(),
            offset,
            slots: f.slots as u16,
        });
        offset += f.slots as u16;
    }
    Ok(out)
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

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
mod tap;

pub use ir::Func;
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
    let funcs = lower::lower_program(&file)?;
    let (code, symbols) = codegen::codegen_program(&funcs, ORG, None);
    Ok(Program { code, symbols })
}

/// Compile a program and wrap it as a bootable `.tap` that runs from `entry`
/// (a function name, default `"main"`). The autoloader `CLEAR`s below [`ORG`],
/// `LOAD`s the code there, and `RANDOMIZE USR`s the entry.
pub fn compile_to_tap(src: &str, entry: &str, name: &str) -> Result<Vec<u8>, String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let funcs = lower::lower_program(&file)?;
    if !funcs.iter().any(|(n, _)| n == entry) {
        return Err(format!("no `{entry}` function"));
    }
    // Emit a DI/EI trampoline at ORG and boot into it (`USR ORG`).
    let (code, _) = codegen::codegen_program(&funcs, ORG, Some(entry));
    Ok(to_tap(&code, ORG, ORG, name))
}

/// The pure-target SDK prelude (dialect source), prepended when compiling an
/// `impl Game`. `Frame`/`Input` method calls lower to these functions; they draw
/// straight into screen RAM via `poke`/`peek` — the host counterparts live in
/// `speccy-sdk`. (`Colour` mirrors the SDK's non-bright variants 0..7.)
const PRELUDE: &str = r#"
enum Colour {
    Black = 0, Blue = 1, Red = 2, Magenta = 3, Green = 4, Cyan = 5, Yellow = 6, White = 7,
    BrightBlue = 9, BrightRed = 10, BrightMagenta = 11, BrightGreen = 12,
    BrightCyan = 13, BrightYellow = 14, BrightWhite = 15
}
enum Button { Up = 1, Down = 2, Left = 4, Right = 8, Fire = 16 }
fn __px_addr(x: u16, y: u16) -> u16 {
    16384u16 + (y / 64u16) * 2048u16 + (y % 8u16) * 256u16 + ((y / 8u16) % 8u16) * 32u16 + x / 8u16
}
fn __px_mask(x: u16) -> u16 {
    let m = [128u8, 64u8, 32u8, 16u8, 8u8, 4u8, 2u8, 1u8];
    m[(x % 8u16) as usize] as u16
}
fn __frame_pixel(x: u16, y: u16, on: u16) {
    let a = __px_addr(x, y);
    let mask = __px_mask(x);
    if on == 0u16 {
        poke(a, peek(a) & (255u16 ^ mask));
    } else {
        poke(a, peek(a) | mask);
    }
}
fn __frame_clear(colour: u16) {
    let attr = colour * 8u16 + 7u16;
    let mut p = 16384u16;
    while p < 22528u16 { poke(p, 0u8); p = p + 1u16; }
    while p < 23296u16 { poke(p, attr as u8); p = p + 1u16; }
}
fn __key(port: u16, bit: u16) -> u16 {
    let mut r = 0u16;
    if (inport(port) & bit) == 0u16 { r = 1u16; }
    r
}
fn __input_held(b: u16) -> u16 {
    let mut h = 0u16;
    if b == 1u16  { h = __key(61438u16, 8u16)  | __key(64510u16, 1u16); }
    if b == 2u16  { h = __key(61438u16, 16u16) | __key(65022u16, 1u16); }
    if b == 4u16  { h = __key(63486u16, 16u16) | __key(57342u16, 2u16); }
    if b == 8u16  { h = __key(61438u16, 4u16)  | __key(57342u16, 1u16); }
    if b == 16u16 { h = __key(61438u16, 1u16)  | __key(32766u16, 1u16); }
    h
}
"#;

/// Compile an `impl Game for T` to a bootable `.tap`. The *same* source also
/// compiles under `rustc` against `speccy-sdk` (host) — this is the pure target:
/// `T`'s state is a zero-initialised global, and a generated frame loop calls
/// `T::update(&state, …)` each frame, with `Frame`/`Input` calls routed to the
/// [`PRELUDE`].
pub fn compile_game(src: &str, name: &str) -> Result<Vec<u8>, String> {
    Ok(compile_game_with_symbols(src, name)?.0)
}

/// One typed source → two artifacts (spec 08): a bootable `.tap` *and* the
/// [`SymbolMap`] — the bridge that lets an env read the game's typed fields off
/// the running tape's RAM. The map reflects the exact layout codegen uses, so a
/// field's emitted address is where it actually lives at runtime.
pub fn compile_game_with_symbols(src: &str, name: &str) -> Result<(Vec<u8>, SymbolMap), String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let ty = find_game_impl(&file).ok_or("no `impl Game for T` found")?;
    let layout = struct_layout(&file, &ty).ok_or_else(|| format!("struct `{ty}` not found"))?;
    let symbols = codegen::state_symbols(&layout); // codegen owns the RAM layout
    let state_bytes = symbols.size; // total state, in bytes

    let combined = format!("{PRELUDE}\n{src}");
    let cfile: syn::File = syn::parse_str(&combined).map_err(|e| format!("parse error: {e}"))?;
    let funcs = lower::lower_program(&cfile)?;
    let update = format!("{ty}::update");
    if !funcs.iter().any(|(n, _)| *n == update) {
        return Err(format!("`{ty}` has no `update` method"));
    }
    let code = codegen::codegen_game(&funcs, ORG, &update, state_bytes);
    Ok((to_tap(&code, ORG, ORG, name), symbols))
}

/// Does the source contain an `impl Game for T` (so [`compile_game`] applies)?
pub fn has_game(src: &str) -> bool {
    syn::parse_str::<syn::File>(src)
        .ok()
        .and_then(|f| find_game_impl(&f))
        .is_some()
}

fn find_game_impl(file: &syn::File) -> Option<String> {
    for item in &file.items {
        if let syn::Item::Impl(imp) = item {
            if let Some((_, path, _)) = &imp.trait_ {
                if path.is_ident("Game") {
                    if let syn::Type::Path(p) = &*imp.self_ty {
                        return p.path.get_ident().map(|i| i.to_string());
                    }
                }
            }
        }
    }
    None
}

/// The game-state struct's layout as `(field_name, slot_count)` in declaration
/// order — a scalar is 1 slot, a `[u16; N]` array field is `N` slots.
fn struct_layout(file: &syn::File, name: &str) -> Option<Vec<(String, usize)>> {
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            if s.ident == name {
                if let syn::Fields::Named(n) = &s.fields {
                    let mut out = Vec::new();
                    for f in &n.named {
                        let fname = f.ident.as_ref().unwrap().to_string();
                        let slots = match &f.ty {
                            syn::Type::Array(arr) => array_len(&arr.len)?,
                            _ => 1,
                        };
                        out.push((fname, slots));
                    }
                    return Some(out);
                }
            }
        }
    }
    None
}

/// The `N` in a `[T; N]` array type's length expression (a literal).
fn array_len(e: &syn::Expr) -> Option<usize> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<usize>().ok();
        }
    }
    None
}

/// The RAM location of one game-state field on the running tape. `count` is 1 for a
/// scalar field and `N` for a `[u16; N]` array field (so an env can read/inject the
/// whole field with `count` × `width` bytes from `addr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub field: String,
    pub addr: u16,
    pub width: u8,
    pub count: u16,
    pub ty: String,
}

/// The compiler-emitted symbol map (spec 08 §2): the game-state struct's RAM
/// layout, derived from the same constant field offsets codegen uses. It is the
/// bridge that carries types across the dial — an env reads a `.tap`'s typed fields
/// off Z80 RAM via this map, with no hand-written addresses. The full layout is
/// always emitted (never a curated subset) so an env can reconstruct any `Self`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolMap {
    /// Where the state struct instance lives in RAM (the compiler's fixed base,
    /// `0xB000`) — read this rather than hardcoding the address.
    pub base: u16,
    /// Total state size in bytes.
    pub size: u16,
    pub fields: Vec<Symbol>,
}

impl SymbolMap {
    /// Address of a named field, if present.
    pub fn addr_of(&self, field: &str) -> Option<u16> {
        self.fields
            .iter()
            .find(|f| f.field == field)
            .map(|f| f.addr)
    }

    /// Render as a `.sym.toml` sidecar (the artifact written next to the `.tap`).
    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# emitted by rustz80 from the Game state struct layout — never hand-written\n",
        );
        s.push_str("[state]\n");
        s.push_str(&format!("base = 0x{:04X}\n", self.base));
        s.push_str(&format!("size = {}\n\n", self.size));
        s.push_str("[fields]\n");
        for f in &self.fields {
            s.push_str(&format!(
                "\"{}\" = {{ addr = 0x{:04X}, width = {}, count = {}, ty = \"{}\" }}\n",
                f.field, f.addr, f.width, f.count, f.ty
            ));
        }
        s
    }
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

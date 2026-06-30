//! Compile an SDK `impl Game` to a bootable `.tap` + a [`SymbolMap`] — the game
//! layer on top of the generic [`rustz80`] compiler (spec 08). The *same* source
//! runs under `rustc` against this crate (host) and compiles here (pure): `T`'s
//! state is a zero-initialised region at [`GAME_STATE`], a generated frame loop
//! calls `T::update(&state, …)` each frame, and `Frame`/`Input` method calls route
//! to the dialect [`PRELUDE`]. Behind the `compile` feature so runtime consumers
//! don't pull in `rustz80`/`syn`.

use crate::symbols::{Symbol, SymbolMap};

/// Where a `Game`'s single global state instance lives on the pure tape (well above
/// the compiler's per-function scratch). The SDK owns this ABI; the symbol map
/// echoes it as `base`.
pub const GAME_STATE: u16 = 0xB000;

/// The pure-target SDK prelude (dialect source), prepended when compiling an
/// `impl Game`. `Frame`/`Input` method calls route to these functions; they draw
/// straight into screen RAM via `poke`/`peek`. Mirrors this crate's runtime
/// `Colour`/`Button`/`Frame`/`Input` — kept here, next to the types it mirrors.
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
fn __frame_fill_cell(cx: u16, cy: u16, colour: u16) {
    // Solid 8x8 block in `colour` ink on black paper (mirrors `Frame::fill_cell`).
    let ink = colour % 8u16;
    let bright = (colour / 8u16) % 2u16;
    let attr = bright * 64u16 + ink;
    let x = cx * 8u16;
    let y = cy * 8u16;
    let mut r = 0u16;
    while r < 8u16 {
        poke(__px_addr(x, y + r), 255u8);
        r = r + 1u16;
    }
    poke(22528u16 + cy * 32u16 + cx, attr as u8);
}
fn __frame_clear_cell(cx: u16, cy: u16) {
    // Blank the 8x8 block; leave the attribute as-is (mirrors `Frame::clear_cell`).
    let x = cx * 8u16;
    let y = cy * 8u16;
    let mut r = 0u16;
    while r < 8u16 {
        poke(__px_addr(x, y + r), 0u8);
        r = r + 1u16;
    }
}
fn __glyph(cx: u16, cy: u16, code: u16) {
    // Blit one ROM-font glyph (8x8) into cell (cx, cy). The 48K font is at 0x3D00,
    // 8 bytes per char from code 32 — read it with `peek` (no const-data needed).
    let base = 15616u16 + (code - 32u16) * 8u16;
    let x = cx * 8u16;
    let y = cy * 8u16;
    let mut r = 0u16;
    while r < 8u16 {
        poke(__px_addr(x, y + r), peek(base + r) as u8);
        r = r + 1u16;
    }
    poke(22528u16 + cy * 32u16 + cx, 7u8); // white ink on black paper
}
fn __frame_number(cx: u16, cy: u16, n: u16) {
    // `Frame::text_u16` on the pure tape: draw `n` as decimal via the ROM font, most-
    // significant digit first. A shared routine — DCE (rustz80 0.6+) prunes it from
    // games that never call `text_u16`, so it costs only games that use it.
    let mut div = 1u16;
    while (n / div) >= 10u16 {
        div = div * 10u16;
    }
    let mut v = n;
    let mut i = 0u16;
    loop {
        __glyph(cx + i, cy, (v / div) % 10u16 + 48u16);
        v = v % div;
        div = div / 10u16;
        i = i + 1u16;
        if div == 0u16 {
            break;
        }
    }
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

/// The `Frame`/`Input` method → prelude-fn routing the lowerer needs (it drops the
/// receiver; see `rustz80::PreludeConfig`).
fn prelude_config() -> rustz80::PreludeConfig {
    rustz80::PreludeConfig::new()
        .route("Frame", "pixel", "__frame_pixel")
        .route("Frame", "clear", "__frame_clear")
        .route("Frame", "fill_cell", "__frame_fill_cell")
        .route("Frame", "clear_cell", "__frame_clear_cell")
        .route("Frame", "text_u16", "__frame_number")
        .route("Input", "held", "__input_held")
        .route("Input", "pressed", "__input_held")
}

/// Compile an `impl Game for T` to a bootable `.tap`.
pub fn compile_game(src: &str, name: &str) -> Result<Vec<u8>, String> {
    Ok(compile_game_with_symbols(src, name)?.0)
}

/// One typed source → two artifacts (spec 08): a bootable `.tap` *and* the
/// [`SymbolMap`] — the bridge that lets an env read the game's typed fields off the
/// running tape's RAM. The map reflects the exact layout codegen uses.
pub fn compile_game_with_symbols(src: &str, name: &str) -> Result<(Vec<u8>, SymbolMap), String> {
    let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
    let ty = find_game_impl(&file).ok_or("no `impl Game for T` found")?;
    let layout = struct_layout(&file, &ty).ok_or_else(|| format!("struct `{ty}` not found"))?;
    let symbols = build_symbols(&layout);
    let state_bytes = symbols.size;

    let combined = format!("{PRELUDE}\n{src}");
    let cfile: syn::File = syn::parse_str(&combined).map_err(|e| format!("parse error: {e}"))?;
    let funcs = rustz80::lower_program(&cfile, &prelude_config())?;
    let update = format!("{ty}::update");
    if !funcs.iter().any(|(n, _)| *n == update) {
        return Err(format!("`{ty}` has no `update` method"));
    }
    // 0.8: `codegen_loop` returns `Err` if the game's code + locals won't fit (over budget)
    // instead of silently emitting a broken tape — surface it as a compile error.
    let code = rustz80::codegen_loop(&funcs, rustz80::ORG, &update, GAME_STATE, state_bytes)?;
    Ok((
        rustz80::to_tap(&code, rustz80::ORG, rustz80::ORG, name),
        symbols,
    ))
}

/// Does the source contain an `impl Game for T` (so [`compile_game`] applies)?
pub fn has_game(src: &str) -> bool {
    syn::parse_str::<syn::File>(src)
        .ok()
        .and_then(|f| find_game_impl(&f))
        .is_some()
}

/// Assign each field an address from [`GAME_STATE`] (consecutive `u16` slots; an
/// array or tuple field of `N` slots reserves `N` elements). `ty` records the source
/// type so a consumer can tell a `[u16; N]` array from a `(u16, u16)` tuple.
fn build_symbols(layout: &[(String, usize, String)]) -> SymbolMap {
    let mut fields = Vec::with_capacity(layout.len());
    let mut slot = 0usize;
    for (name, slots, ty) in layout {
        fields.push(Symbol {
            field: name.clone(),
            addr: GAME_STATE + (slot as u16) * 2,
            width: 2,
            count: *slots as u16,
            ty: ty.clone(),
        });
        slot += *slots;
    }
    SymbolMap {
        base: GAME_STATE,
        size: (slot as u16) * 2,
        fields,
    }
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

/// The game-state struct's layout as `(field_name, slot_count, type_string)` in
/// declaration order — a scalar is 1 slot, a `[u16; N]` array field is `N` slots, and
/// a `(u16, …)` tuple field is one slot per element.
fn struct_layout(file: &syn::File, name: &str) -> Option<Vec<(String, usize, String)>> {
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            if s.ident == name {
                if let syn::Fields::Named(n) = &s.fields {
                    let mut out = Vec::new();
                    for f in &n.named {
                        let fname = f.ident.as_ref().unwrap().to_string();
                        let slots = match &f.ty {
                            syn::Type::Array(arr) => array_len(&arr.len)?,
                            syn::Type::Tuple(t) => t.elems.len(),
                            _ => 1,
                        };
                        out.push((fname, slots, type_string(&f.ty)));
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

/// A compact source-type string for the symbol map's `ty`: `u16` / `[u16; 8]` /
/// `(u16, u16)` / a named type. Good enough to round-trip and to distinguish an
/// array field from a tuple field.
fn type_string(t: &syn::Type) -> String {
    match t {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "u16".into()),
        syn::Type::Array(arr) => {
            let len = array_len(&arr.len).unwrap_or(0);
            format!("[{}; {}]", type_string(&arr.elem), len)
        }
        syn::Type::Tuple(tup) => {
            let parts: Vec<String> = tup.elems.iter().map(type_string).collect();
            format!("({})", parts.join(", "))
        }
        _ => "u16".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_map_handles_tuple_and_array_fields() {
        // The state layout reflects tuple fields (one slot per element) and arrays,
        // with a `ty` that distinguishes them — and the whole map round-trips.
        let src = "struct State { score: u16, head: (u16, u16), body: [u16; 8] }";
        let file: syn::File = syn::parse_str(src).unwrap();
        let layout = struct_layout(&file, "State").expect("layout");
        let map = build_symbols(&layout);

        assert_eq!(map.base, GAME_STATE);
        assert_eq!(map.size, (1 + 2 + 8) * 2); // 11 slots → 22 bytes

        let head = map.fields.iter().find(|f| f.field == "head").unwrap();
        assert_eq!((head.count, head.ty.as_str()), (2, "(u16, u16)"));
        assert_eq!(head.addr, GAME_STATE + 2); // after `score` (1 slot)

        let body = map.fields.iter().find(|f| f.field == "body").unwrap();
        assert_eq!((body.count, body.ty.as_str()), (8, "[u16; 8]"));
        assert_eq!(body.addr, GAME_STATE + 6); // after score(1) + head(2) = 3 slots

        assert_eq!(SymbolMap::from_toml(&map.to_toml()).unwrap(), map);
    }

    #[test]
    fn solid_cell_draw_compiles_pure() {
        // The data-free cell primitives (`fill_cell`/`clear_cell`, colour by value)
        // route through the prelude and compile to a bootable tape — Snake's sprite
        // path with no tile data to relocate. No `Rng` here, so this isolates the
        // draw routing. (Args are u8 to also match the host `Frame` signature.)
        let src = r#"
#[derive(Default)]
struct Walk { x: u8, started: u8 }
impl Game for Walk {
    fn update(&mut self, input: &Input, frame: &mut Frame) {
        if self.started == 0u8 { frame.clear(Colour::Black); self.started = 1u8; }
        frame.clear_cell(self.x, 5u8);
        if input.held(Button::Right) { self.x = self.x + 1u8; }
        frame.fill_cell(self.x, 5u8, Colour::BrightGreen);
    }
}
"#;
        assert!(has_game(src));
        compile_game(src, "WALK").expect("solid-cell game compiles pure");
    }
}

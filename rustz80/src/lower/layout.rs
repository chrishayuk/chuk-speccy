//! Struct/enum *layout* and the small syntactic helpers for reading the accepted
//! subset out of `syn`. Layout is a lowering-only concern: every struct field has a
//! constant slot offset, so `s.field` lowers to a plain slot — codegen stays unaware.

use crate::ir::Width;
use std::collections::HashMap;

/// One struct field's layout: its name and slot count (1 for a scalar, `N` for a
/// `[u16; N]` array field, `N × sizeof(Cell)` for a `[Cell; N]` struct-element array).
/// Offsets are the running sum of `slots`. `elem_struct` names the element struct of a
/// `[Cell; N]` field, so element access (`s.field[i].x`) knows its stride + sub-layout.
#[derive(Clone)]
pub(crate) struct FieldDef {
    pub(crate) name: String,
    pub(crate) slots: usize,
    pub(crate) elem_struct: Option<String>,
}

/// Struct layouts: name → fields in declaration order. A field occupies `slots`
/// consecutive `u16` slots; a field's offset is the sum of preceding `slots`.
pub(crate) type Structs = HashMap<String, Vec<FieldDef>>;

/// C-like enum layouts: name → variants as `(name, value)`. Values follow Rust's
/// rule: an explicit `= N` discriminant, else the previous value + 1 (from 0).
pub(crate) type Enums = HashMap<String, Vec<(String, u16)>>;

/// Total `u16` slots a struct occupies.
pub(crate) fn struct_slots(fields: &[FieldDef]) -> usize {
    fields.iter().map(|f| f.slots).sum()
}

/// A field's slot offset: the running sum of preceding fields' slots (so array
/// fields shift everything after them by their length).
pub(crate) fn field_offset(fields: &[FieldDef], name: &str) -> Result<usize, String> {
    let mut off = 0;
    for f in fields {
        if f.name == name {
            return Ok(off);
        }
        off += f.slots;
    }
    Err(format!("no field {name}"))
}

pub(crate) fn collect_enums(file: &syn::File) -> Result<Enums, String> {
    let mut m = Enums::new();
    for item in &file.items {
        if let syn::Item::Enum(e) = item {
            let mut variants = Vec::new();
            let mut next = 0u16;
            for v in &e.variants {
                let value = match &v.discriminant {
                    Some((_, expr)) => lit_u16(expr)?,
                    None => next,
                };
                variants.push((v.ident.to_string(), value));
                next = value.wrapping_add(1);
            }
            m.insert(e.ident.to_string(), variants);
        }
    }
    Ok(m)
}

pub(crate) fn collect_structs(file: &syn::File) -> Result<Structs, String> {
    let mut m = Structs::new();
    let no_consts = HashMap::new();
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            // Const-generic structs (`[T; N]` sized by a const param) have a per-instance
            // layout — handled by the generics machinery, not collected eagerly here.
            if s.generics.const_params().next().is_some() {
                continue;
            }
            let syn::Fields::Named(named) = &s.fields else {
                return Err(format!(
                    "only named-field structs are supported: {}",
                    s.ident
                ));
            };
            // Compute with the structs sized so far (element structs must precede).
            let name = s.ident.to_string();
            let fields = struct_field_defs(named, &no_consts, &m, &name)?;
            m.insert(name, fields);
        }
    }
    Ok(m)
}

/// Lay out one named field: `(slots, elem_struct)`. A `[u16; N]` field is `N` slots; a
/// `[Cell; N]` field is `N × sizeof(Cell)` slots with `elem_struct = Some("Cell")`
/// (`structs` supplies element sizes — the element struct must be defined earlier); a
/// tuple `(u16, …)` is one slot per element; everything else is a single slot. A
/// `[u16; N]` length may be a const-generic parameter, resolved via `consts`.
fn field_def(
    f: &syn::Field,
    consts: &HashMap<String, u16>,
    structs: &Structs,
    owner: &str,
) -> Result<FieldDef, String> {
    let name = f.ident.as_ref().unwrap().to_string();
    let (slots, elem_struct) = match &f.ty {
        syn::Type::Path(_) => (1, None),
        syn::Type::Array(arr) if is_u16(&arr.elem) => (array_len(&arr.len, consts)?, None),
        // `[Cell; N]` — an array of structs.
        syn::Type::Array(arr) => {
            let elem = match &*arr.elem {
                syn::Type::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                _ => None,
            };
            let elem = elem.ok_or_else(|| {
                format!("array field `{owner}.{name}` element must be `u16` or a struct")
            })?;
            let esize = structs.get(&elem).map(|f| struct_slots(f)).ok_or_else(|| {
                format!("array field element `{elem}` of `{owner}.{name}` must be `u16` or a previously-defined struct")
            })?;
            (array_len(&arr.len, consts)? * esize, Some(elem))
        }
        // A tuple field `(u16, u16)` occupies one slot per (scalar) element, accessed
        // by `.0` / `.1`.
        syn::Type::Tuple(t) => {
            if !t.elems.iter().all(|e| matches!(e, syn::Type::Path(_))) {
                return Err(format!(
                    "tuple struct fields must have scalar elements: {owner}"
                ));
            }
            (t.elems.len(), None)
        }
        _ => {
            return Err(format!(
                "only scalar, array, or tuple struct fields are supported: {owner}"
            ))
        }
    };
    Ok(FieldDef {
        name,
        slots,
        elem_struct,
    })
}

/// An array length: a literal, or a const-generic parameter resolved via `consts`.
fn array_len(e: &syn::Expr, consts: &HashMap<String, u16>) -> Result<usize, String> {
    if let syn::Expr::Path(p) = e {
        if let Some(id) = p.path.get_ident() {
            if let Some(n) = consts.get(&id.to_string()) {
                return Ok(*n as usize);
            }
        }
    }
    lit_len(e)
}

/// The field layout of a named-field struct, resolving const-param array lengths and
/// struct-element array sizes (`structs` supplies element sizes).
pub(crate) fn struct_field_defs(
    named: &syn::FieldsNamed,
    consts: &HashMap<String, u16>,
    structs: &Structs,
    owner: &str,
) -> Result<Vec<FieldDef>, String> {
    named
        .named
        .iter()
        .map(|f| field_def(f, consts, structs, owner))
        .collect()
}

/// `Enum::Variant` (a 2-segment path) → its integer value, if known.
pub(crate) fn resolve_enum_path(path: &syn::Path, enums: &Enums) -> Option<u16> {
    if path.segments.len() != 2 {
        return None;
    }
    let name = path.segments[0].ident.to_string();
    let variant = path.segments[1].ident.to_string();
    enums
        .get(&name)?
        .iter()
        .find(|(n, _)| *n == variant)
        .map(|(_, v)| *v)
}

/// The base name of an `impl` target type — the last path segment's ident, so both
/// `impl Foo` and a generic `impl<T> Pair<T>` (or `Pair<u16>`) resolve to the struct
/// name. Type arguments are erased: every `T`-typed field is a 16-bit slot, so a
/// generic struct shares one layout (like any struct's fields), and its methods are
/// lowered once.
pub(crate) fn type_name(t: &syn::Type) -> Result<String, String> {
    if let syn::Type::Path(p) = t {
        if let Some(seg) = p.path.segments.last() {
            return Ok(seg.ident.to_string());
        }
    }
    Err(format!("unsupported impl type: {t:?}"))
}

pub(crate) fn member_name(m: &syn::Member) -> Result<String, String> {
    match m {
        syn::Member::Named(n) => Ok(n.to_string()),
        syn::Member::Unnamed(_) => Err("tuple-struct fields not supported".into()),
    }
}

/// Element width inferred from an initialiser value's literal suffix (`0u8` →
/// byte; everything else → word). Good enough for byte arrays.
pub(crate) fn elem_width(e: &syn::Expr) -> Width {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            if i.suffix() == "u8" {
                return Width::Byte;
            }
        }
    }
    Width::Word
}

pub(crate) fn lit_len(e: &syn::Expr) -> Result<usize, String> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<usize>().map_err(|e| e.to_string());
        }
    }
    Err("array length must be an integer literal".into())
}

fn lit_u16(e: &syn::Expr) -> Result<u16, String> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<u16>().map_err(|e| e.to_string());
        }
    }
    Err("enum discriminant must be an integer literal".into())
}

/// Is `ty` the `u16` path type?
fn is_u16(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(p) if p.path.is_ident("u16"))
}

//! Struct/enum *layout* and the small syntactic helpers for reading the accepted
//! subset out of `syn`. Layout is a lowering-only concern: every struct field has a
//! constant slot offset, so `s.field` lowers to a plain slot — codegen stays unaware.

use crate::ir::Width;
use std::collections::HashMap;

/// One struct field's layout: its name and slot count (1 for a scalar, `N` for a
/// `[u16; N]` array field). Offsets are the running sum of `slots`.
#[derive(Clone)]
pub(crate) struct FieldDef {
    pub(crate) name: String,
    pub(crate) slots: usize,
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
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            let syn::Fields::Named(named) = &s.fields else {
                return Err(format!(
                    "only named-field structs are supported: {}",
                    s.ident
                ));
            };
            let mut fields = Vec::new();
            for f in &named.named {
                // Scalar fields are one slot; `[u16; N]` array fields are N slots.
                // Other shapes would mislay offsets, so reject them clearly.
                let slots = match &f.ty {
                    syn::Type::Path(_) => 1,
                    syn::Type::Array(arr) if is_u16(&arr.elem) => lit_len(&arr.len)?,
                    syn::Type::Array(_) => {
                        return Err(format!(
                            "only `[u16; N]` array struct fields are supported: {}",
                            s.ident
                        ))
                    }
                    _ => {
                        return Err(format!(
                            "only scalar or `[u16; N]` struct fields are supported: {}",
                            s.ident
                        ))
                    }
                };
                fields.push(FieldDef {
                    name: f.ident.as_ref().unwrap().to_string(),
                    slots,
                });
            }
            m.insert(s.ident.to_string(), fields);
        }
    }
    Ok(m)
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

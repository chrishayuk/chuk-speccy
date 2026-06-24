//! The symbol map (spec 08 §2): the game-state struct's RAM layout — the bridge that
//! lets an env read/inject a `.tap`'s typed fields off Z80 RAM with no hand-written
//! addresses. The compiler ([`crate::compile`], `compile` feature) emits it next to
//! the tape as `<game>.sym.toml`; consumers (the env) parse it back here. This type
//! has no `syn` dependency, so it lives in the default SDK.

/// One field's RAM location. `count` is 1 for a scalar field and `N` for a `[u16; N]`
/// array field (read/inject the whole field with `count` × `width` bytes from `addr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub field: String,
    pub addr: u16,
    pub width: u8,
    pub count: u16,
    pub ty: String,
}

/// A game-state layout: the struct base, total size, and every field's location.
/// The *full* layout is always present (spec 08 §2) so an env can reconstruct any
/// `Self`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolMap {
    /// Where the state struct instance lives in RAM (read this, don't hardcode it).
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
            "# emitted by the SDK from the Game state struct layout — never hand-written\n",
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

    /// Parse a `.sym.toml` sidecar. Deliberately tolerant of our own fixed format
    /// rather than pulling in a full TOML parser.
    pub fn from_toml(src: &str) -> Result<SymbolMap, String> {
        let mut map = SymbolMap::default();
        let mut section = "";
        for line in src.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                section = match name {
                    "state" => "state",
                    "fields" => "fields",
                    _ => "",
                };
                continue;
            }
            match section {
                "state" => {
                    if let Some(v) = kv(line, "base") {
                        map.base = parse_int(v)?;
                    } else if let Some(v) = kv(line, "size") {
                        map.size = parse_int(v)?;
                    }
                }
                "fields" => map.fields.push(parse_field(line)?),
                _ => {}
            }
        }
        Ok(map)
    }
}

fn parse_int(s: &str) -> Result<u16, String> {
    let s = s.trim();
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        s.parse::<u16>()
    };
    v.map_err(|_| format!("bad integer {s:?}"))
}

/// Split a `key = value` line; returns the value if the key matches.
fn kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once('=')?;
    (k.trim() == key).then(|| v.trim())
}

/// Parse `"name" = { addr = 0x.., width = N, count = N, ty = ".." }`.
fn parse_field(line: &str) -> Result<Symbol, String> {
    let field = line
        .split('"')
        .nth(1)
        .ok_or_else(|| format!("no field name in {line:?}"))?
        .to_string();
    let inner = line
        .split_once('{')
        .and_then(|(_, r)| r.split_once('}'))
        .map(|(b, _)| b)
        .ok_or_else(|| format!("no {{ }} body in {line:?}"))?;
    // `ty` is a quoted string that may itself contain commas (e.g. a tuple
    // `"(u16, u16)"`), so pull it out before splitting the rest on commas.
    let ty = inner
        .split_once("ty")
        .and_then(|(_, r)| r.split_once('"'))
        .and_then(|(_, r)| r.split_once('"'))
        .map(|(t, _)| t.to_string())
        .unwrap_or_else(|| "u16".to_string());
    let (mut addr, mut width, mut count) = (0u16, 2u8, 1u16);
    for part in inner.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "addr" => addr = parse_int(v)?,
                "width" => width = parse_int(v)? as u8,
                "count" => count = parse_int(v)?,
                _ => {}
            }
        }
    }
    Ok(Symbol {
        field,
        addr,
        width,
        count,
        ty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let m = SymbolMap {
            base: 0xB000,
            size: 6,
            fields: vec![
                Symbol {
                    field: "score".into(),
                    addr: 0xB000,
                    width: 2,
                    count: 1,
                    ty: "u16".into(),
                },
                Symbol {
                    field: "cells".into(),
                    addr: 0xB002,
                    width: 2,
                    count: 8,
                    ty: "u16".into(),
                },
            ],
        };
        let back = SymbolMap::from_toml(&m.to_toml()).expect("parse");
        assert_eq!(back, m);
        assert_eq!(back.addr_of("cells"), Some(0xB002));
        assert_eq!(back.fields[1].count, 8);
        assert_eq!(back.addr_of("nope"), None);
    }

    #[test]
    fn tuple_field_ty_round_trips() {
        // A tuple field's `ty` contains a comma — it must survive the TOML round-trip
        // (the parser pulls `ty` out before splitting the rest on commas).
        let m = SymbolMap {
            base: 0xB000,
            size: 6,
            fields: vec![
                Symbol {
                    field: "head".into(),
                    addr: 0xB000,
                    width: 2,
                    count: 2,
                    ty: "(u16, u16)".into(),
                },
                Symbol {
                    field: "score".into(),
                    addr: 0xB004,
                    width: 2,
                    count: 1,
                    ty: "u16".into(),
                },
            ],
        };
        let back = SymbolMap::from_toml(&m.to_toml()).expect("parse");
        assert_eq!(back, m);
        assert_eq!(back.fields[0].ty, "(u16, u16)");
        assert_eq!(back.fields[0].count, 2);
    }
}

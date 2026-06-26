//! The compiled, cacheable cell artifact — `CellProgram` + its image format.
use super::config::check_caps;
use super::report::sorted_symbols;
use super::*;
use crate::Program;
use std::collections::HashMap;

/// A **compiled** cell: the result of parse + lower + codegen under a policy. Cheap to
/// clone and cache (e.g. by source hash) — re-running a known snippet then skips the
/// (syn-parse-dominated, ~16 µs) compile. Turn one into a runnable machine with
/// [`Runner::new`].
#[derive(Clone)]
pub struct CellProgram {
    pub(super) prog: Program,
    pub(super) cfg: CellConfig,
}

impl CellProgram {
    /// Compile `src` with the **permissive** policy (raw memory + ports allowed, no
    /// ceilings) — for trusted/game code.
    pub fn compile(src: &str) -> Result<Self, String> {
        Self::compile_with_config(src, CellConfig::permissive())
    }

    /// Compile `src` under `cfg`: enforce its capability gates (`poke`/`peek`/`inport`)
    /// and `max_code_bytes`. Parses once (shared by the cap scan and the compile).
    pub fn compile_with_config(src: &str, cfg: CellConfig) -> Result<Self, String> {
        let file: syn::File = syn::parse_str(src).map_err(|e| format!("parse error: {e}"))?;
        check_caps(&file, &cfg)?;
        // The cell runs in Cell80 mode: `*`/`/`/`%` lower to `ED FE` host traps that the
        // bus services natively (no software mul/div runtime appended).
        let prog = crate::compile_file(&file, crate::Target::Cell)?;
        if let Some(max) = cfg.max_code_bytes {
            if prog.code.len() > max {
                return Err(format!(
                    "code is {} bytes, over the {max}-byte limit",
                    prog.code.len()
                ));
            }
        }
        Ok(CellProgram { prog, cfg })
    }

    /// The underlying program (symbol map, code).
    pub fn program(&self) -> &Program {
        &self.prog
    }

    /// Serialize to a compact, self-contained **image** — code + symbols + policy, no syn,
    /// no source. Cache it (by hash), ship it, retrieve it; [`from_bytes`](Self::from_bytes)
    /// reloads it in ~µs, skipping the parse-dominated (~16 µs) compile. The cell
    /// "cartridge": a few dozen bytes you can hash, index, and hand around cheaply.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(IMAGE_MAGIC);
        b.push(IMAGE_VER);
        b.extend_from_slice(&(self.prog.code.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.prog.code);
        let syms = sorted_symbols(&self.prog.symbols); // deterministic → stable hash
        b.extend_from_slice(&(syms.len() as u16).to_le_bytes());
        for (name, addr) in &syms {
            b.push(name.len() as u8);
            b.extend_from_slice(name.as_bytes());
            b.extend_from_slice(&addr.to_le_bytes());
        }
        let c = &self.cfg;
        let flags = (c.allow_raw_memory as u8)
            | (c.allow_ports as u8) << 1
            | (c.max_code_bytes.is_some() as u8) << 2
            | (c.max_touched.is_some() as u8) << 3;
        b.push(flags);
        b.extend_from_slice(&(c.max_code_bytes.unwrap_or(0) as u32).to_le_bytes());
        b.extend_from_slice(&(c.max_touched.unwrap_or(0) as u32).to_le_bytes());
        b
    }

    /// Reload an image written by [`to_bytes`](Self::to_bytes) — no parse, no compile.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut r = ImageReader { b: bytes, i: 0 };
        if r.take(4)? != IMAGE_MAGIC {
            return Err("not a CZ80 cell image".into());
        }
        let ver = r.u8()?;
        if ver != IMAGE_VER {
            return Err(format!("unsupported cell-image version {ver}"));
        }
        let code_len = r.u16()? as usize;
        let code = r.take(code_len)?.to_vec();
        let nsym = r.u16()?;
        let mut symbols = HashMap::with_capacity(nsym as usize);
        for _ in 0..nsym {
            let nlen = r.u8()? as usize;
            let name = std::str::from_utf8(r.take(nlen)?)
                .map_err(|_| "bad symbol name in image")?
                .to_string();
            symbols.insert(name, r.u16()?);
        }
        let flags = r.u8()?;
        let max_code = r.u32()? as usize;
        let max_touched = r.u32()? as usize;
        Ok(CellProgram {
            prog: Program { code, symbols },
            cfg: CellConfig {
                allow_raw_memory: flags & 1 != 0,
                allow_ports: flags & 2 != 0,
                max_code_bytes: (flags & 4 != 0).then_some(max_code),
                max_touched: (flags & 8 != 0).then_some(max_touched),
            },
        })
    }
}

const IMAGE_MAGIC: &[u8; 4] = b"CZ80";

const IMAGE_VER: u8 = 1;

/// A tiny bounds-checked byte cursor — shared by [`CellProgram::from_bytes`] and the
/// `.cell` cartridge reader ([`super::cartridge`]).
pub(super) struct ImageReader<'a> {
    pub(super) b: &'a [u8],
    pub(super) i: usize,
}
impl<'a> ImageReader<'a> {
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).ok_or("cell image truncated")?;
        let s = self.b.get(self.i..end).ok_or("cell image truncated")?;
        self.i = end;
        Ok(s)
    }
    pub(super) fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    pub(super) fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub(super) fn u64(&mut self) -> Result<u64, String> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes(s.try_into().unwrap()))
    }
    /// A `u16`-length-prefixed UTF-8 string.
    pub(super) fn string(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        Ok(std::str::from_utf8(self.take(n)?)
            .map_err(|_| "bad utf-8 in cell image")?
            .to_string())
    }
}

/// Write a `u16`-length-prefixed UTF-8 string into `b` (mirror of [`ImageReader::string`]).
pub(super) fn put_string(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

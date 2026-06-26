//! The `.cell` cartridge — a named, versioned, **self-describing** tool artifact: a
//! [`Manifest`] (id / summary / tags / entry / source-hash / compiler+ABI version) wrapping
//! a compiled [`CellProgram`] image. This is the portable object the CLI, a tool index, and
//! the MCP server pass around — the gate for "compile once → ship → discover → run."
use super::program::{put_string, ImageReader};
use super::report::sorted_symbols;
use super::*;
use crate::Signature;
use std::hash::{Hash, Hasher};

const MAGIC: &[u8; 4] = b"CELL";
const VERSION: u8 = 2; // v2 added the typed I/O signature

/// Serialize / read a `(name, type)` pair list (signature params / state fields).
fn put_pairs(b: &mut Vec<u8>, v: &[(String, String)]) {
    b.extend_from_slice(&(v.len() as u16).to_le_bytes());
    for (n, t) in v {
        put_string(b, n);
        put_string(b, t);
    }
}
fn read_pairs(r: &mut ImageReader) -> Result<Vec<(String, String)>, String> {
    let n = r.u16()?;
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push((r.string()?, r.string()?));
    }
    Ok(v)
}

/// Self-describing metadata carried by a `.cell` cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// A stable identifier (e.g. `"grid.manhattan.v1"`; defaults to the entry name).
    pub id: String,
    /// One-line human/agent summary (for a tool index).
    pub summary: String,
    /// Free-form tags for search/filtering.
    pub tags: Vec<String>,
    /// The default entry to run.
    pub entry: String,
    /// A **non-cryptographic** hash of the source (provenance / cache key).
    pub source_hash: u64,
    /// The `rustz80` version that produced this cartridge.
    pub compiler_version: String,
    /// The [`ABI_VERSION`] the cartridge targets.
    pub abi_version: u32,
    /// The typed I/O signature of the entry — so a registry/MCP can present the interface
    /// and validate named inputs **without re-parsing** the source.
    pub signature: Signature,
}

/// Options for [`Cartridge::compile`] (all optional).
#[derive(Default)]
pub struct CartridgeOpts {
    pub id: Option<String>,
    pub entry: Option<String>,
    pub summary: String,
    pub tags: Vec<String>,
}

/// A compiled cell **plus** its manifest — the `.cell` artifact.
#[derive(Clone)]
pub struct Cartridge {
    pub manifest: Manifest,
    pub program: CellProgram,
}

impl Cartridge {
    /// Compile `src` under `cfg` and wrap it in a cartridge: resolves the entry (opts, then
    /// `run`/`main`), hashes the source, and stamps the compiler + ABI versions.
    pub fn compile(src: &str, cfg: CellConfig, opts: CartridgeOpts) -> Result<Self, String> {
        let program = CellProgram::compile_with_config(src, cfg)?;
        let syms = &program.program().symbols;
        let entry = match opts.entry {
            Some(e) if syms.contains_key(&e) => e,
            Some(e) => return Err(format!("no entry `{e}` in the program")),
            None if syms.contains_key("run") => "run".into(),
            None if syms.contains_key("main") => "main".into(),
            None => return Err("no `run`/`main` entry — pass an explicit entry".into()),
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut h);
        let signature = crate::entry_signature(src, &entry)?;
        Ok(Cartridge {
            manifest: Manifest {
                id: opts.id.unwrap_or_else(|| entry.clone()),
                summary: opts.summary,
                tags: opts.tags,
                entry,
                source_hash: h.finish(),
                compiler_version: env!("CARGO_PKG_VERSION").to_string(),
                abi_version: ABI_VERSION,
                signature,
            },
            program,
        })
    }

    /// Serialize to `.cell` bytes (manifest + the [`CellProgram`] image).
    pub fn to_bytes(&self) -> Vec<u8> {
        let m = &self.manifest;
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(VERSION);
        b.extend_from_slice(&m.abi_version.to_le_bytes());
        put_string(&mut b, &m.id);
        put_string(&mut b, &m.summary);
        b.extend_from_slice(&(m.tags.len() as u16).to_le_bytes());
        for t in &m.tags {
            put_string(&mut b, t);
        }
        put_string(&mut b, &m.entry);
        b.extend_from_slice(&m.source_hash.to_le_bytes());
        put_string(&mut b, &m.compiler_version);
        put_pairs(&mut b, &m.signature.params);
        put_string(&mut b, &m.signature.ret);
        put_pairs(&mut b, &m.signature.state);
        let img = self.program.to_bytes();
        b.extend_from_slice(&(img.len() as u32).to_le_bytes());
        b.extend_from_slice(&img);
        b
    }

    /// Reload a `.cell` cartridge — no parse, no compile.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut r = ImageReader { b: bytes, i: 0 };
        if r.take(4)? != MAGIC {
            return Err("not a .cell cartridge".into());
        }
        let ver = r.u8()?;
        if ver != VERSION {
            return Err(format!("unsupported .cell version {ver}"));
        }
        let abi_version = r.u32()?;
        let id = r.string()?;
        let summary = r.string()?;
        let ntags = r.u16()?;
        let mut tags = Vec::with_capacity(ntags as usize);
        for _ in 0..ntags {
            tags.push(r.string()?);
        }
        let entry = r.string()?;
        let source_hash = r.u64()?;
        let compiler_version = r.string()?;
        let signature = Signature {
            params: read_pairs(&mut r)?,
            ret: r.string()?,
            state: read_pairs(&mut r)?,
        };
        let img_len = r.u32()? as usize;
        let program = CellProgram::from_bytes(r.take(img_len)?)?;
        Ok(Cartridge {
            manifest: Manifest {
                id,
                summary,
                tags,
                entry,
                source_hash,
                compiler_version,
                abi_version,
                signature,
            },
            program,
        })
    }

    /// A human-readable inspection summary.
    pub fn to_human(&self) -> String {
        let m = &self.manifest;
        let p = self.program.program();
        let c = &self.program.cfg;
        let entry_addr = p.symbols.get(&m.entry).copied().unwrap_or(0);
        let caps = format!(
            "raw_memory={} ports={} max_code={} max_touched={}",
            c.allow_raw_memory,
            c.allow_ports,
            c.max_code_bytes.map_or("∞".into(), |n| n.to_string()),
            c.max_touched.map_or("∞".into(), |n| n.to_string()),
        );
        let syms: Vec<String> = sorted_symbols(&p.symbols)
            .iter()
            .map(|(n, a)| format!("{n}@0x{a:04X}"))
            .collect();
        let state = if m.signature.state.is_empty() {
            String::new()
        } else {
            let fs: Vec<String> = m
                .signature
                .state
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect();
            format!("\n  state: {{ {} }}", fs.join(", "))
        };
        format!(
            "cell `{}`  (abi {}, compiler {})\n  {}\n  tags: {}\n  signature: {}{}\n  \
             entry: {} @ 0x{:04X}\n  code: {} bytes, {} functions\n  capabilities: {}\n  \
             symbols: {}\n  source_hash: 0x{:016x}",
            m.id,
            m.abi_version,
            m.compiler_version,
            if m.summary.is_empty() {
                "(no summary)"
            } else {
                &m.summary
            },
            if m.tags.is_empty() {
                "—".into()
            } else {
                m.tags.join(", ")
            },
            m.signature.to_decl(&m.entry),
            state,
            m.entry,
            entry_addr,
            p.code.len(),
            p.size_report().len(),
            caps,
            syms.join(", "),
            m.source_hash,
        )
    }

    /// A JSON inspection summary (for tooling / a tool index).
    pub fn to_json(&self) -> String {
        let m = &self.manifest;
        let p = self.program.program();
        let c = &self.program.cfg;
        let tags: Vec<String> = m.tags.iter().map(|t| format!("\"{t}\"")).collect();
        let syms: Vec<String> = sorted_symbols(&p.symbols)
            .iter()
            .map(|(n, a)| format!("\"{n}\":{a}"))
            .collect();
        let pairs_json = |v: &[(String, String)]| -> String {
            v.iter()
                .map(|(n, t)| format!("[\"{n}\",\"{t}\"]"))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{{\"id\":\"{}\",\"abi\":{},\"compiler\":\"{}\",\"summary\":\"{}\",\"tags\":[{}],\
             \"entry\":\"{}\",\"signature\":{{\"params\":[{}],\"ret\":\"{}\",\"state\":[{}]}},\
             \"code_bytes\":{},\"functions\":{},\"source_hash\":\"0x{:016x}\",\
             \"capabilities\":{{\"raw_memory\":{},\"ports\":{},\"max_code\":{},\"max_touched\":{}}},\
             \"symbols\":{{{}}}}}",
            m.id,
            m.abi_version,
            m.compiler_version,
            m.summary,
            tags.join(","),
            m.entry,
            pairs_json(&m.signature.params),
            m.signature.ret,
            pairs_json(&m.signature.state),
            p.code.len(),
            p.size_report().len(),
            m.source_hash,
            c.allow_raw_memory,
            c.allow_ports,
            c.max_code_bytes.map_or("null".into(), |n| n.to_string()),
            c.max_touched.map_or("null".into(), |n| n.to_string()),
            syms.join(","),
        )
    }
}

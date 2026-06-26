//! CLI for the `rustz80-cell` binary: argument parsing + `run_cli`.
use super::*;

/// CLI usage line, shared by the `rustz80-cell` binary.
pub const USAGE: &str = "usage:\n  \
     rustz80-cell run <file.rs> [--entry NAME] [--cycles N] [--args a,b,c] \
     [--set addr:ty=val,...] [--read name@addr:ty,...] [--json]\n  \
     rustz80-cell compile <file.rs> -o <file.cell> [--entry NAME] [--id ID] \
     [--summary TEXT] [--tags a,b] [safety]\n  \
     rustz80-cell inspect <file.cell> [--json]\n  \
     rustz80-cell index <dir>                 (list the cell library in <dir>)\n  \
     rustz80-cell search <query> <dir>        (rank library cells by relevance)\n  \
     safety (sandboxed by default): [--allow-raw-memory] [--allow-ports] \
     [--max-code-bytes N] [--max-touched N]";

/// Parse a comma-separated arg list — decimal or `0x`-prefixed hex, each a `u16`.
pub fn parse_args(s: &str) -> Result<Vec<u16>, String> {
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let v = match t.strip_prefix("0x") {
                Some(h) => u16::from_str_radix(h, 16),
                None => t.parse::<u16>(),
            };
            v.map_err(|_| format!("bad arg `{t}` (want a u16, decimal or 0x..)"))
        })
        .collect()
}

/// Parse a `--set` spec — comma-separated `addr:ty=value` (addr/value decimal or `0x..`),
/// the typed inputs written into memory before the run.
fn parse_sets(s: &str) -> Result<Vec<(u16, Ty, u64)>, String> {
    let num16 = |t: &str| match t.strip_prefix("0x") {
        Some(h) => u16::from_str_radix(h, 16),
        None => t.parse::<u16>(),
    };
    let num64 = |t: &str| match t.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => t.parse::<u64>(),
    };
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let bad = || format!("bad --set `{t}` (want addr:ty=value)");
            let (lhs, val_s) = t.split_once('=').ok_or_else(bad)?;
            let (addr_s, ty_s) = lhs.split_once(':').ok_or_else(bad)?;
            let addr = num16(addr_s).map_err(|_| format!("bad address in `{t}`"))?;
            let val = num64(val_s).map_err(|_| format!("bad value in `{t}`"))?;
            Ok((addr, Ty::parse(ty_s)?, val))
        })
        .collect()
}

/// Parse a `--read` spec — comma-separated `name@addr:ty` (addr decimal or `0x..`).
fn parse_reads(s: &str) -> Result<Vec<(String, u16, Ty)>, String> {
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let t = t.trim();
            let bad = || format!("bad --read `{t}` (want name@addr:ty)");
            let (name, rest) = t.split_once('@').ok_or_else(bad)?;
            let (addr_s, ty_s) = rest.split_once(':').ok_or_else(bad)?;
            let addr = match addr_s.strip_prefix("0x") {
                Some(h) => u16::from_str_radix(h, 16),
                None => addr_s.parse::<u16>(),
            }
            .map_err(|_| format!("bad address in `{t}`"))?;
            Ok((name.to_string(), addr, Ty::parse(ty_s)?))
        })
        .collect()
}

/// Dispatch `run` / `compile` / `inspect` and return the formatted output. The
/// `rustz80-cell` binary is a shim over this.
pub fn run_cli(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("compile") => cmd_compile(&args[1..]),
        Some("inspect") => cmd_inspect(&args[1..]),
        Some("index") => cmd_index(&args[1..]),
        Some("search") => cmd_search(&args[1..]),
        Some(other) => Err(format!("unknown command `{other}`\n{USAGE}")),
        None => Err(USAGE.into()),
    }
}

/// Parse a cell source's leading `//!` header → `(summary, tags, entry)`.
fn parse_meta(src: &str) -> (String, Vec<String>, Option<String>) {
    let (mut summary, mut tags, mut entry) = (String::new(), Vec::new(), None);
    for line in src.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("//!") {
            let rest = rest.trim();
            if let Some(t) = rest.strip_prefix("tags:") {
                tags = t
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            } else if let Some(e) = rest.strip_prefix("entry:") {
                entry = Some(e.trim().to_string());
            } else if summary.is_empty() {
                summary = rest.to_string();
            }
        } else if !l.is_empty() && !l.starts_with("//") {
            break; // first code line — header done
        }
    }
    (summary, tags, entry)
}

/// Build a cartridge from a library `.rs` (id = file stem, metadata from the `//!` header)
/// or load a `.cell`. Returns `None` for any other extension.
fn library_cartridge(path: &std::path::Path) -> Option<Result<Cartridge, String>> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some((|| {
            let src =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let (summary, tags, entry) = parse_meta(&src);
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("cell")
                .to_string();
            Cartridge::compile(
                &src,
                CellConfig::sandboxed(),
                CartridgeOpts {
                    id: Some(id),
                    entry,
                    summary,
                    tags,
                },
            )
        })()),
        Some("cell") => Some(
            std::fs::read(path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|b| Cartridge::from_bytes(&b)),
        ),
        _ => None,
    }
}

/// Build an index over every cell (`.rs` / `.cell`) in `dir`, sorted by id.
fn index_dir(dir: &str) -> Result<CellIndex, String> {
    let mut idx = CellIndex::new();
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        if let Some(c) = library_cartridge(&path) {
            idx.add(c?.manifest);
        }
    }
    Ok(idx)
}

fn render(m: &crate::cell::Manifest) -> String {
    format!(
        "  {} — {}  [{}]  ({})",
        m.id,
        if m.summary.is_empty() {
            "(no summary)"
        } else {
            &m.summary
        },
        m.tags.join(", "),
        m.signature.to_decl(&m.entry),
    )
}

/// `index <dir>` — list the cell library (in id order).
fn cmd_index(args: &[String]) -> Result<String, String> {
    let dir = args.first().ok_or(USAGE)?;
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        if let Some(c) = library_cartridge(&path) {
            rows.push(render(&c?.manifest));
        }
    }
    Ok(format!(
        "cell library `{dir}` ({} cells):\n{}",
        rows.len(),
        rows.join("\n")
    ))
}

/// `search <query> <dir>` — rank the library by relevance to `query`.
fn cmd_search(args: &[String]) -> Result<String, String> {
    let query = args.first().ok_or(USAGE)?;
    let dir = args.get(1).ok_or("search needs a directory")?;
    let idx = index_dir(dir)?;
    let hits = idx.search(query, 10);
    let mut out = format!(
        "indexed {} cells; query `{query}` → {} match(es):\n",
        idx.len(),
        hits.len()
    );
    for m in hits {
        out += &render(m);
        out.push('\n');
    }
    Ok(out)
}

/// `compile <file.rs> -o <file.cell> [--entry] [--id] [--summary] [--tags] [safety]` —
/// compile source to a `.cell` cartridge on disk; print the inspection summary.
fn cmd_compile(args: &[String]) -> Result<String, String> {
    let mut it = args.iter();
    let file = it.next().ok_or(USAGE)?;
    let mut out: Option<String> = None;
    let mut opts = CartridgeOpts::default();
    let mut cfg = CellConfig::sandboxed();
    let num = |o: Option<&String>, what: &str| -> Result<usize, String> {
        o.ok_or_else(|| format!("{what} needs a number"))?
            .parse()
            .map_err(|_| format!("bad {what}"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = Some(it.next().ok_or("-o needs a path")?.clone()),
            "--entry" => opts.entry = Some(it.next().ok_or("--entry needs a name")?.clone()),
            "--id" => opts.id = Some(it.next().ok_or("--id needs a value")?.clone()),
            "--summary" => opts.summary = it.next().ok_or("--summary needs text")?.clone(),
            "--tags" => {
                opts.tags = it
                    .next()
                    .ok_or("--tags needs a list")?
                    .split(',')
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| t.trim().to_string())
                    .collect()
            }
            "--allow-raw-memory" => cfg.allow_raw_memory = true,
            "--allow-ports" => cfg.allow_ports = true,
            "--max-code-bytes" => cfg.max_code_bytes = Some(num(it.next(), "--max-code-bytes")?),
            "--max-touched" => cfg.max_touched = Some(num(it.next(), "--max-touched")?),
            other => return Err(format!("unknown option `{other}`\n{USAGE}")),
        }
    }
    let out = out.ok_or("compile needs an output path: -o <file.cell>")?;
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let cart = Cartridge::compile(&src, cfg, opts)?;
    std::fs::write(&out, cart.to_bytes()).map_err(|e| format!("{out}: {e}"))?;
    Ok(format!("wrote {out}\n{}", cart.to_human()))
}

/// `inspect <file.cell> [--json]` — load a cartridge and print its manifest/symbols/caps.
fn cmd_inspect(args: &[String]) -> Result<String, String> {
    let mut it = args.iter();
    let file = it.next().ok_or(USAGE)?;
    let json = it.any(|a| a == "--json");
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let cart = Cartridge::from_bytes(&bytes)?;
    Ok(if json {
        cart.to_json()
    } else {
        cart.to_human()
    })
}

/// `run <file.rs> [opts]` — compile source and run it, returning the report (JSON if
/// `--json`, else the human summary).
fn cmd_run(args: &[String]) -> Result<String, String> {
    let mut it = args.iter();
    let file = it.next().ok_or(USAGE)?;
    let mut entry: Option<String> = None;
    let mut cycles = DEFAULT_CYCLES;
    let mut call_args: Vec<u16> = Vec::new();
    let mut sets: Vec<(u16, Ty, u64)> = Vec::new();
    let mut reads: Vec<(String, u16, Ty)> = Vec::new();
    let mut json = false;
    let mut cfg = CellConfig::sandboxed(); // safe by default on the CLI
    let num = |o: Option<&String>, what: &str| -> Result<usize, String> {
        o.ok_or_else(|| format!("{what} needs a number"))?
            .parse()
            .map_err(|_| format!("bad {what}"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--entry" => entry = Some(it.next().ok_or("--entry needs a name")?.clone()),
            "--cycles" => {
                cycles = it
                    .next()
                    .ok_or("--cycles needs a number")?
                    .parse()
                    .map_err(|_| "bad --cycles (want a positive integer)")?
            }
            "--args" => call_args = parse_args(it.next().ok_or("--args needs values")?)?,
            "--set" => sets = parse_sets(it.next().ok_or("--set needs a spec")?)?,
            "--read" => reads = parse_reads(it.next().ok_or("--read needs a spec")?)?,
            "--allow-raw-memory" => cfg.allow_raw_memory = true,
            "--allow-ports" => cfg.allow_ports = true,
            "--max-code-bytes" => cfg.max_code_bytes = Some(num(it.next(), "--max-code-bytes")?),
            "--max-touched" => cfg.max_touched = Some(num(it.next(), "--max-touched")?),
            "--json" => json = true,
            other => return Err(format!("unknown option `{other}`\n{USAGE}")),
        }
    }
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let mut runner = Runner::compile_with_config(&src, cfg)?;
    let mut report = runner.run_with_inputs(entry.as_deref(), &call_args, &sets, cycles)?;
    if !reads.is_empty() {
        report.reads = runner.read_named(&reads); // decode typed fields from post-run memory
    }
    Ok(if json {
        report.to_json()
    } else {
        report.to_human()
    })
}

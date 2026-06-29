//! `speccy-asset` — turn a PNG into Spectrum art, with an attribute-clash report.
//!
//! ```text
//! speccy-asset <art.png> [-o art.scr]            # full 256×192 screen -> .scr blob
//! speccy-asset bake <sprite.png> [--name HERO] [-o sprite.rs]
//!                                                # any 8-multiple size -> const Tile(s)
//! ```
//!
//! `scr` (the default/bare form) makes a runtime screen blob; `bake` makes **authored
//! game data** — `const Tile` definitions that drop into a `speccy-sdk` game's
//! `frame.tile(..)`. Both print which cells/tiles clash on real hardware. Loads
//! RGB/RGBA/grayscale 8-bit PNGs.

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            help();
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: no input PNG (try --help)");
            ExitCode::FAILURE
        }
        Some("bake") => run_bake(&args[1..]),
        Some("scr") => run_scr(&args[1..]),
        _ => run_scr(&args), // bare form: treat the first arg as the input PNG
    }
}

fn help() {
    eprintln!(
        "usage:\n  \
         speccy-asset <art.png> [-o art.scr]                 256x192 screen -> .scr\n  \
         speccy-asset bake <sprite.png> [--name HERO] [-o sprite.rs]\n                                                     any 8-multiple size -> const Tile(s)"
    );
}

/// `scr` / bare form: a full 256×192 image to a 6912-byte `.scr` + cell clash report.
fn run_scr(args: &[String]) -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => output = it.next().cloned(),
            other => input = Some(other.to_string()),
        }
    }
    let Some(input) = input else {
        eprintln!("error: no input PNG (try --help)");
        return ExitCode::FAILURE;
    };
    let out_path = output.unwrap_or_else(|| replace_ext(&input, "scr"));

    let (rgb, w, h) = match load_png(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot load {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let img = match speccy_assets::convert(&rgb, w, h) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e} — resize the source to 256x192 first");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, img.to_scr()) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }

    println!("wrote {out_path} (6912-byte .scr)");
    let total = speccy_assets::COLS * speccy_assets::ROWS;
    report_clashes(
        img.clashes.len(),
        total,
        "cells",
        img.clashes
            .iter()
            .map(|c| (c.cx as usize, c.cy as usize, Some(c.colours))),
    );
    ExitCode::SUCCESS
}

/// `bake`: an image of any 8-multiple size to `const Tile` Rust + per-tile clash report.
fn run_bake(args: &[String]) -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut name = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => output = it.next().cloned(),
            "-n" | "--name" => name = it.next().cloned(),
            other => input = Some(other.to_string()),
        }
    }
    let Some(input) = input else {
        eprintln!("error: no input PNG (try --help)");
        return ExitCode::FAILURE;
    };
    let stem = stem(&input);
    let out_path = output.unwrap_or_else(|| replace_ext(&input, "rs"));
    let name = name.unwrap_or_else(|| stem.to_string());

    let (rgb, w, h) = match load_png(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot load {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let sheet = match speccy_assets::bake::bake(&rgb, w, h) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&out_path, sheet.to_rust(&name)) {
        eprintln!("error: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }

    println!(
        "wrote {out_path} ({}×{} tiles, {}×{}px)",
        sheet.cols, sheet.rows, w, h
    );
    report_clashes(
        sheet.clashes(),
        sheet.tiles.len(),
        "tiles",
        sheet
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.clash)
            .map(|(i, _)| (i % sheet.cols, i / sheet.cols, None)),
    );
    ExitCode::SUCCESS
}

/// Print the shared clash summary (up to 12 offenders), in cells or tiles.
fn report_clashes(
    n: usize,
    total: usize,
    unit: &str,
    offenders: impl Iterator<Item = (usize, usize, Option<u8>)>,
) {
    if n == 0 {
        println!("no attribute clashes — clean on real hardware");
        return;
    }
    println!("{n} / {total} {unit} clash (wanted >2 colours):");
    for (i, (x, y, colours)) in offenders.enumerate() {
        if i == 12 {
            println!("  … and {} more", n - 12);
            break;
        }
        match colours {
            Some(c) => println!("  ({x:>2},{y:>2}) wanted {c} colours"),
            None => println!("  tile ({x:>2},{y:>2})"),
        }
    }
}

/// Decode an 8-bit PNG to row-major RGB triples. Palette/16-bit/`tRNS` are normalised
/// to 8-bit channels; alpha is dropped, grayscale is replicated across RGB.
fn load_png(path: &str) -> Result<(Vec<[u8; 3]>, usize, usize), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..info.buffer_size()];
    let (w, h) = (info.width as usize, info.height as usize);

    let rgb: Vec<[u8; 3]> = match info.color_type {
        png::ColorType::Rgb => data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        png::ColorType::Rgba => data.chunks_exact(4).map(|c| [c[0], c[1], c[2]]).collect(),
        png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => {
            data.chunks_exact(2).map(|c| [c[0], c[0], c[0]]).collect()
        }
        other => return Err(format!("unsupported PNG colour type {other:?}")),
    };
    Ok((rgb, w, h))
}

/// The filename stem (`art/hero.png` → `hero`), for default output/const names.
fn stem(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.strip_suffix(".png").unwrap_or(file)
}

/// Swap the `.png` extension for `ext` (`hero.png`, `"rs"` → `hero.rs`).
fn replace_ext(path: &str, ext: &str) -> String {
    format!("{}.{ext}", path.strip_suffix(".png").unwrap_or(path))
}

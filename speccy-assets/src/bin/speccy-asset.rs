//! `speccy-asset` — convert a PNG to a ZX Spectrum `.scr` and print its attribute
//! clashes.
//!
//! ```text
//! speccy-asset art.png [-o art.scr]
//! ```
//!
//! The input must be **256×192** (the Spectrum screen); resize the source first.
//! Loads RGB/RGBA/grayscale 8-bit PNGs. The `.scr` drops straight into any emulator.

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => output = args.next(),
            "-h" | "--help" => {
                eprintln!("usage: speccy-asset <input.png> [-o out.scr]");
                return ExitCode::SUCCESS;
            }
            other => input = Some(other.to_string()),
        }
    }
    let Some(input) = input else {
        eprintln!("error: no input PNG (try --help)");
        return ExitCode::FAILURE;
    };
    let out_path = output.unwrap_or_else(|| {
        format!("{}.scr", input.strip_suffix(".png").unwrap_or(&input))
    });

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

    // The clash report — the point of the tool.
    let n = img.clashes.len();
    println!("wrote {out_path} (6912-byte .scr)");
    if n == 0 {
        println!("no attribute clashes — clean on real hardware");
    } else {
        let total = speccy_assets::COLS * speccy_assets::ROWS;
        println!("{n} / {total} cells clash (wanted >2 colours):");
        for c in img.clashes.iter().take(12) {
            println!("  cell ({:>2},{:>2}) wanted {} colours", c.cx, c.cy, c.colours);
        }
        if n > 12 {
            println!("  … and {} more", n - 12);
        }
    }
    ExitCode::SUCCESS
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

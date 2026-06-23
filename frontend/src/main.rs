//! A themed terminal (TUI) head over the spectrum core — one of the many heads
//! the display pipeline supports (`docs/05-frontends-display-spec.md`).
//!
//! Two modes, chosen automatically:
//!   * interactive terminal -> a **live** 50 Hz loop: run a frame, redraw with
//!     Unicode block glyphs, map keystrokes to the keyboard matrix. Ctrl-C quits.
//!   * piped / redirected   -> a one-shot ASCII luminance render (survives
//!     copy-paste and non-colour terminals).
//!
//! The live renderer packs sub-character pixels via block glyphs, picking the
//! finest grid that fits the terminal with correct aspect (fractional sampling,
//! no integer-scale waste). `quad` (default) = 2×2 px/char using Block Elements
//! (universal font support). `sextant` = 2×3 px/char (sharper vertically, needs a
//! font with the Legacy Computing block). `half` = the old 1×2.
//!
//! Usage: `speccy <48.rom> [theme] [snapshot.sna|.z80] [quad|sextant|half] [ascii]`
//!   theme: authentic | dark | light | terminal | amber | gameboy  (default authentic)

use display::{BorderMode, DisplayConfig, Frame};
use spectrum::keyboard::{self, KeyPos};
use spectrum::Spectrum;
use std::io::{IsTerminal, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: speccy <48.rom> [theme] [scale] [live|ascii]");
            std::process::exit(2);
        }
    };
    // Parse the remaining args by type (order-independent): a theme name, a
    // snapshot path (.sna/.z80), a render mode, or the `live`/`ascii` flags.
    let mut theme_name = "authentic".to_string();
    let mut snapshot_path: Option<String> = None;
    let mut mode = RenderMode::Quad;
    let mut force_ascii = false;
    let mut force_live = false;
    for a in args {
        match a.as_str() {
            "ascii" => force_ascii = true,
            "live" => force_live = true,
            "quad" => mode = RenderMode::Quad,
            "sextant" | "sext" => mode = RenderMode::Sext,
            "half" | "halfblock" => mode = RenderMode::Half,
            _ if a.ends_with(".sna") || a.ends_with(".z80") || a.ends_with(".tap") => {
                snapshot_path = Some(a)
            }
            _ if DisplayConfig::preset(&a).is_some() => theme_name = a,
            _ => eprintln!("ignoring unrecognised arg '{a}'"),
        }
    }

    let mut cfg = DisplayConfig::preset(&theme_name).unwrap();
    cfg.border = BorderMode::Thin;

    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("could not read ROM {rom_path}: {e}");
        std::process::exit(1);
    });

    let mut spec = Spectrum::new_48k(&rom);
    let mut loaded_game = false;
    if let Some(p) = &snapshot_path {
        match std::fs::read(p) {
            Ok(data) if p.ends_with(".tap") => {
                for _ in 0..250 {
                    spec.run_frame(); // boot, then insert tape + LOAD ""
                }
                match spec.load_tap(&data) {
                    Ok(()) => {
                        spec.autoload_tape();
                        loaded_game = true;
                    }
                    Err(e) => eprintln!("tape load failed: {e:?}"),
                }
            }
            Ok(data) => {
                let fmt = if p.ends_with(".sna") { "sna" } else { "z80" };
                match spec.load_snapshot(fmt, &data) {
                    Ok(()) => loaded_game = true,
                    Err(e) => eprintln!("snapshot load failed: {e:?}"),
                }
            }
            Err(e) => eprintln!("could not read media {p}: {e}"),
        }
    }
    if !loaded_game {
        for _ in 0..250 {
            spec.run_frame(); // boot to the BASIC prompt
        }
    }

    let live = force_live || (std::io::stdout().is_terminal() && !force_ascii);
    if live {
        run_live(&mut spec, &cfg, mode, &theme_name);
    } else {
        if !loaded_game {
            draw_test_card(&mut spec); // give the blank prompt something to show
        }
        let frame = display::render(&spec.screen_indexed(), spec.border(), &cfg);
        print!("{}", to_ascii_shades(&frame, 4));
        println!("theme: {theme_name}  (ASCII mode; run interactively for live + colour)");
    }
}

/// How many sub-pixels each output character packs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    /// 1×2 — Upper-half block. Maximum font compatibility, least detail.
    Half,
    /// 2×2 — Block Elements quadrants. Universal font support; the default.
    Quad,
    /// 2×3 — Legacy Computing sextants. Sharpest vertically; needs a capable font.
    Sext,
}

impl RenderMode {
    /// Sub-grid (cols, rows) of pixels per character cell.
    fn grid(self) -> (usize, usize) {
        match self {
            RenderMode::Half => (1, 2),
            RenderMode::Quad => (2, 2),
            RenderMode::Sext => (2, 3),
        }
    }
    fn label(self) -> &'static str {
        match self {
            RenderMode::Half => "half",
            RenderMode::Quad => "quad",
            RenderMode::Sext => "sextant",
        }
    }
}

/// The live 50 Hz loop: emulate a frame, take input, redraw.
fn run_live(spec: &mut Spectrum, cfg: &DisplayConfig, mode: RenderMode, theme: &str) {
    let _guard = RawMode::enable();

    // Choose the largest aspect-correct character grid that fits the terminal.
    // For the image's pixels to look square: out_cols : out_rows*ASPECT ==
    // frame_w : frame_h, where ASPECT is the terminal cell's height:width. We ask
    // the terminal for its real cell size (so fonts of any proportion render
    // correctly), falling back to a typical 2.1 if it doesn't answer.
    let aspect = std::env::var("SPECCY_ASPECT")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(query_cell_aspect)
        .unwrap_or(2.1);
    let (term_rows, term_cols) = term_size();
    let probe = display::render(&spec.screen_indexed(), spec.border(), cfg);
    let (fw, fh) = (probe.width as f64, probe.height as f64);
    let avail_rows = term_rows.saturating_sub(1).max(1);
    let mut out_rows = avail_rows;
    let mut out_cols = (out_rows as f64 * aspect * fw / fh).floor() as usize;
    if out_cols > term_cols {
        out_cols = term_cols;
        out_rows = (out_cols as f64 * fh / (aspect * fw)).floor() as usize;
    }
    out_cols = out_cols.max(1);
    out_rows = out_rows.max(1).min(avail_rows);

    // Read stdin on a thread; the loop polls the channel so it never blocks.
    let (tx, rx) = mpsc::channel::<u8>();
    thread::spawn(move || {
        let mut byte = [0u8; 1];
        let mut stdin = std::io::stdin();
        while stdin.read(&mut byte).map(|n| n > 0).unwrap_or(false) {
            if tx.send(byte[0]).is_err() {
                break;
            }
        }
    });

    let mut out = std::io::stdout();

    // Terminals send key *down* only, so each keystroke presses for a few frames
    // and auto-releases: (key, optional shift, frames remaining).
    let mut held: Vec<(KeyPos, Option<KeyPos>, u32)> = Vec::new();
    let tick = Duration::from_micros(19_968); // ~50.08 Hz
    let max_ticks: Option<u64> = std::env::var("SPECCY_TICKS")
        .ok()
        .and_then(|s| s.parse().ok());
    let mut n: u64 = 0;

    loop {
        let t0 = Instant::now();

        while let Ok(b) = rx.try_recv() {
            match b {
                3 | 0x1d => return, // Ctrl-C / Ctrl-]
                _ => {
                    if let Some((pos, shift)) = map_byte(b) {
                        spec.set_key(pos, true);
                        if let Some(s) = shift {
                            spec.set_key(s, true);
                        }
                        held.push((pos, shift, 4));
                    }
                }
            }
        }

        spec.run_frame();

        held.retain_mut(|(pos, shift, frames)| {
            *frames -= 1;
            if *frames == 0 {
                spec.set_key(*pos, false);
                if let Some(s) = shift {
                    spec.set_key(*s, false);
                }
                false
            } else {
                true
            }
        });

        let frame = display::render(&spec.screen_indexed(), spec.border(), cfg);
        let mut buf = String::with_capacity(256 * 1024);
        buf.push_str("\x1b[H");
        render_blocks(&mut buf, &frame, out_cols, out_rows, mode);
        // Clear anything below the image, then a status line with no trailing
        // newline (so the bottom row never scrolls the view).
        buf.push_str("\x1b[0m\x1b[J");
        buf.push_str(&format!(
            "{theme} · {} · type to drive BASIC · Ctrl-C quits",
            mode.label()
        ));
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();

        n += 1;
        if max_ticks.is_some_and(|m| n >= m) {
            return;
        }
        let dt = t0.elapsed();
        if dt < tick {
            thread::sleep(tick - dt);
        }
    }
}

/// Map a terminal input byte to a key (and optional shift).
fn map_byte(b: u8) -> Option<(KeyPos, Option<KeyPos>)> {
    match b {
        b'\r' | b'\n' => Some((keyboard::ENTER, None)),
        8 | 127 => Some((KeyPos { row: 4, col: 0 }, Some(keyboard::CAPS_SHIFT))), // DELETE = CAPS+0
        b' ' => Some((keyboard::SPACE, None)),
        _ => keyboard::key_for_char(b as char).map(|(pos, caps, sym)| {
            let shift = if caps {
                Some(keyboard::CAPS_SHIFT)
            } else if sym {
                Some(keyboard::SYM_SHIFT)
            } else {
                None
            };
            (pos, shift)
        }),
    }
}

/// RAII terminal raw mode via `stty` (no external crates). Enters the alternate
/// screen buffer and hides the cursor; `-isig` delivers Ctrl-C as a byte we
/// handle, so teardown always runs and the shell screen is restored intact.
struct RawMode;
impl RawMode {
    fn enable() -> Self {
        stty(&["-echo", "-icanon", "-isig", "min", "1", "time", "0"]);
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J"); // alt screen, hide cursor, clear
        let _ = out.flush();
        RawMode
    }
}
impl Drop for RawMode {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l"); // show cursor, leave alt screen
        let _ = out.flush();
        stty(&["sane"]);
    }
}
fn stty(args: &[&str]) {
    let _ = std::process::Command::new("stty").args(args).status();
}

/// Ask the terminal for its character-cell size in pixels (`CSI 16 t` ->
/// `CSI 6 ; height ; width t`) and return the height:width aspect. Returns None
/// if the terminal doesn't answer within ~0.2s. Must be called in raw mode and
/// *before* any other reader is consuming stdin.
fn query_cell_aspect() -> Option<f64> {
    stty(&["min", "0", "time", "2"]); // read returns after 0.2s even with no data
    {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[16t");
        let _ = out.flush();
    }
    let mut buf = [0u8; 64];
    let n = std::io::stdin().read(&mut buf).unwrap_or(0);
    stty(&["min", "1", "time", "0"]); // restore blocking single-byte reads

    if n == 0 {
        return None;
    }
    // Parse "ESC [ 6 ; H ; W t".
    let txt = String::from_utf8_lossy(&buf[..n]);
    let body = txt
        .trim_matches(|c| c == '\x1b' || c == '[')
        .trim_end_matches('t');
    let parts: Vec<&str> = body.split(';').collect();
    if parts.len() == 3 && parts[0] == "6" {
        if let (Ok(h), Ok(w)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
            if w > 0.0 && h > 0.0 {
                return Some(h / w);
            }
        }
    }
    None
}

/// Terminal size as (rows, cols) via `stty size`, falling back to 24×80.
fn term_size() -> (usize, usize) {
    if let Ok(o) = std::process::Command::new("stty").arg("size").output() {
        if let Ok(s) = String::from_utf8(o.stdout) {
            let mut it = s.split_whitespace();
            if let (Some(Ok(r)), Some(Ok(c))) =
                (it.next().map(str::parse), it.next().map(str::parse))
            {
                return (r, c);
            }
        }
    }
    (24, 80)
}

type Rgb = (u8, u8, u8);

/// Render the frame into an `out_cols × out_rows` grid of block characters. Each
/// character packs a `gx × gy` sub-grid of pixels (per `mode`); the sub-pixels of
/// one cell are reduced to a foreground/background colour pair and a glyph whose
/// set bits mark the foreground pixels. Because the Spectrum has at most 2 colours
/// per 8×8 cell, this is usually exact; near attribute boundaries it degrades to
/// the brightest/darkest pair. Sampling is fractional, so no resolution is wasted.
fn render_blocks(
    s: &mut String,
    frame: &Frame,
    out_cols: usize,
    out_rows: usize,
    mode: RenderMode,
) {
    let (w, h) = (frame.width, frame.height);
    let (gx, gy) = mode.grid();
    let at = |x: usize, y: usize| -> Rgb {
        let p = (y.min(h - 1) * w + x.min(w - 1)) * 4;
        (frame.rgba[p], frame.rgba[p + 1], frame.rgba[p + 2])
    };
    // Sub-pixel sampling: map sub-grid coordinate -> source pixel centre.
    let sx = w as f64 / (out_cols * gx) as f64;
    let sy = h as f64 / (out_rows * gy) as f64;

    let mut samples = [(0u8, 0u8, 0u8); 6]; // up to 2x3
    for cy in 0..out_rows {
        // Sentinel colours so the first cell of each line emits its SGR.
        let mut prev_fg = (1u8, 1u8, 1u8);
        let mut prev_bg = (2u8, 2u8, 2u8);
        for cx in 0..out_cols {
            let n = gx * gy;
            for j in 0..gy {
                for i in 0..gx {
                    let x = (((cx * gx + i) as f64 + 0.5) * sx) as usize;
                    let y = (((cy * gy + j) as f64 + 0.5) * sy) as usize;
                    samples[j * gx + i] = at(x, y);
                }
            }
            let (glyph, fg, bg) = reduce_cell(&samples[..n], gx);
            // Only re-emit SGR when the colour pair changes (smaller frames).
            if fg != prev_fg || bg != prev_bg {
                s.push_str(&format!(
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                    fg.0, fg.1, fg.2, bg.0, bg.1, bg.2
                ));
                prev_fg = fg;
                prev_bg = bg;
            }
            s.push(glyph);
        }
        s.push_str("\x1b[0m\r\n");
    }
}

/// Reduce a cell's sub-pixels to (glyph, fg, bg). fg = brightest sample, bg =
/// darkest; each sub-pixel is "set" (foreground) if it's nearer fg than bg. The
/// glyph encodes which sub-pixels are set, in row-major order.
fn reduce_cell(samples: &[Rgb], gx: usize) -> (char, Rgb, Rgb) {
    let luma = |c: Rgb| 77 * c.0 as u32 + 150 * c.1 as u32 + 29 * c.2 as u32;
    let mut fg = samples[0];
    let mut bg = samples[0];
    let (mut fl, mut bl) = (luma(samples[0]), luma(samples[0]));
    for &c in &samples[1..] {
        let l = luma(c);
        if l > fl {
            fl = l;
            fg = c;
        }
        if l < bl {
            bl = l;
            bg = c;
        }
    }
    // Flat cell: nothing to draw, just a solid background.
    if fl - bl < 24 * 256 {
        return (' ', fg, bg);
    }
    let mid = (fl + bl) / 2;
    let mut mask = 0u8;
    for (k, &c) in samples.iter().enumerate() {
        if luma(c) >= mid {
            mask |= 1 << k;
        }
    }
    let glyph = match (samples.len(), gx) {
        (2, 1) => HALF_GLYPHS[(mask & 0b11) as usize],
        (4, 2) => QUAD_GLYPHS[(mask & 0b1111) as usize],
        (6, 2) => sextant_glyph(mask & 0b111111),
        _ => '█',
    };
    (glyph, fg, bg)
}

// Half block, bits: row0, row1 (top, bottom).
const HALF_GLYPHS: [char; 4] = [' ', '\u{2580}', '\u{2584}', '\u{2588}'];

// Quadrant, bits (row-major): TL=1, TR=2, BL=4, BR=8.
const QUAD_GLYPHS: [char; 16] = [
    ' ', '\u{2598}', '\u{259D}', '\u{2580}', // ., TL, TR, TL+TR
    '\u{2596}', '\u{258C}', '\u{259E}', '\u{259B}', // BL, TL+BL, TR+BL, TL+TR+BL
    '\u{2597}', '\u{259A}', '\u{2590}', '\u{259C}', // BR, TL+BR, TR+BR, TL+TR+BR
    '\u{2584}', '\u{2599}', '\u{259F}', '\u{2588}', // BL+BR, +TL, +TR, full
];

/// Sextant glyph (2×3) for a 6-bit row-major mask. Blank, the two half columns,
/// and full block live outside the Legacy Computing block, so they're special.
fn sextant_glyph(mask: u8) -> char {
    match mask {
        0 => ' ',
        0b010101 => '\u{258C}', // left column (TL,ML,BL) -> left half block
        0b101010 => '\u{2590}', // right column -> right half block
        0b111111 => '\u{2588}', // full
        m => {
            // U+1FB00.. enumerates masks 1..=62 in order, skipping 21 and 42.
            let mut idx = m as u32 - 1;
            if m > 21 {
                idx -= 1;
            }
            if m > 42 {
                idx -= 1;
            }
            char::from_u32(0x1FB00 + idx).unwrap_or('█')
        }
    }
}

/// Render a frame as ASCII luminance shading (no colour) — survives copy-paste.
fn to_ascii_shades(frame: &Frame, scale: usize) -> String {
    const RAMP: &[u8] = b" .:-=+*#%@";
    let (w, h) = (frame.width, frame.height);
    let lum = |x: usize, y: usize| {
        let p = (y.min(h - 1) * w + x.min(w - 1)) * 4;
        (77 * frame.rgba[p] as u32 + 150 * frame.rgba[p + 1] as u32 + 29 * frame.rgba[p + 2] as u32)
            >> 8
    };
    let mut s = String::new();
    for y in (0..h).step_by(scale * 2) {
        for x in (0..w).step_by(scale) {
            let idx = (lum(x, y) as usize * (RAMP.len() - 1)) / 255;
            s.push(RAMP[idx] as char);
        }
        s.push('\n');
    }
    s
}

/// Paint a 16-colour test card into screen RAM (for the static themes demo).
fn draw_test_card(spec: &mut Spectrum) {
    let mut pixels = [0u8; 6144];
    for cell_row in 16..24usize {
        for line in 0..8usize {
            let y = cell_row * 8 + line;
            for col in 0..32usize {
                pixels[spectrum::ula::Ula::pixel_row_addr(y, col)] = 0xAA;
            }
        }
    }
    spec.write_memory(0x4000, &pixels);

    let mut attrs = [0u8; 768];
    for row in 0..24usize {
        for col in 0..32usize {
            let colour = (col / 4) as u8 & 7;
            let bright = if row >= 12 { 0x40 } else { 0 };
            attrs[row * 32 + col] = bright | (colour << 3) | 0x07; // paper=colour, ink=white
        }
    }
    spec.write_memory(0x5800, &attrs);
}

//! `speccy-gif` — render a game/session to an animated GIF: the ffmpeg-free,
//! shareable sibling of the MP4 recorder. A thin reuse of [`display::gif`] over the
//! headless `Machine` — handy for READMEs, agent episode captures, and bug repros.
//!
//! ```text
//! speccy-gif <48.rom> <game.tap|.sna|.z80> <out.gif> [frames=120] [every=2] [boot=420]
//! ```
//! `frames` GIF frames, sampled every `every` emulated frames, after `boot` frames
//! of warm-up (boot + tape load + the title settling).

use spectrum::Spectrum;
use std::process::ExitCode;

fn main() -> ExitCode {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: speccy-gif <48.rom> <game.tap|.sna|.z80> <out.gif> [frames=120] [every=2] [boot=420]");
        return ExitCode::FAILURE;
    }
    let rom = match std::fs::read(&a[0]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read ROM {}: {e}", a[0]);
            return ExitCode::FAILURE;
        }
    };
    let data = match std::fs::read(&a[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot read game {}: {e}", a[1]);
            return ExitCode::FAILURE;
        }
    };
    let out = &a[2];
    let frames: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);
    let every: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(2).max(1);
    let boot: usize = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(420);

    let fmt = if a[1].ends_with(".tap") {
        "tap"
    } else if a[1].ends_with(".tzx") {
        "tzx"
    } else if a[1].ends_with(".sna") {
        "sna"
    } else {
        "z80"
    };

    let mut spec = Spectrum::new_48k(&rom);
    load(&mut spec, fmt, &data);
    while spec.tape_playing() {
        spec.run_frame();
    }
    for _ in 0..boot {
        spec.run_frame();
    }

    let mut gif_frames = Vec::with_capacity(frames);
    for _ in 0..frames {
        for _ in 0..every {
            spec.run_frame();
        }
        gif_frames.push(spec.screen_indexed());
    }

    // 50 Hz → an `every`-frame gap is `every*2` centiseconds of playback.
    let delay_cs = (every * 2) as u16;
    let bytes =
        display::gif::encode_indexed_to_vec(&gif_frames, 256, 192, &display::AUTHENTIC, delay_cs);
    if let Err(e) = std::fs::write(out, &bytes) {
        eprintln!("cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {out} ({frames} frames, {} bytes)", bytes.len());
    ExitCode::SUCCESS
}

fn load(spec: &mut Spectrum, fmt: &str, data: &[u8]) {
    match fmt {
        "tap" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            if spec.load_tap(data).is_ok() {
                spec.autoload_tape();
            }
        }
        "tzx" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            spec.autoload_tape();
            let _ = spec.play_tape("tzx", data);
        }
        _ => {
            let _ = spec.load_snapshot(fmt, data);
        }
    }
}

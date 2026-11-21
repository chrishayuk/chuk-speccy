//! Drive `island` through the *whole* quest — fetch the axe, clear the vine, cross to
//! room 1, fetch the key, open the door, reach the treasure — on the real 48K ROM, and
//! capture the run to an animated GIF. Unlike `speccy-run` (which just boots and idles,
//! no input), this actually presses the keys, so it's the "does the game work" demo.
//!
//! ```text
//! cargo run -p chuk-speccy-sdk --features compile --example island_quest_gif -- \
//!     testroms/48.rom island_quest.gif
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(rom_path) = args.next() else {
        eprintln!("usage: island_quest_gif <48.rom> [out.gif]");
        return ExitCode::FAILURE;
    };
    let out = args.next().unwrap_or_else(|| "island_quest.gif".to_string());

    let rom = match std::fs::read(&rom_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read ROM {rom_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let src = include_str!("../samples/island.rs");
    let (tap, sym) = match speccy_sdk::compile::compile_game_with_symbols(src, "ISLAND") {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: compile: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame();
    }
    if let Err(e) = spec.load_tap(&tap) {
        eprintln!("error: load tape: {e:?}");
        return ExitCode::FAILURE;
    }
    spec.autoload_tape();
    for _ in 0..400 {
        spec.run_frame();
    }

    let read_u16 = |s: &spectrum::Spectrum, field: &str| -> u16 {
        let addr = sym.addr_of(field).expect("field in the symbol map");
        let m = s.read_memory(addr, 2);
        u16::from_le_bytes([m[0], m[1]])
    };

    let mut frames: Vec<Vec<u8>> = vec![spec.screen_indexed()];
    // Hold `ch` until `field` reads as `target` (or `max_frames` elapse), capturing a
    // screen every other emulated frame throughout — the same technique
    // speccy-sdk/tests/dial.rs's `hold_until` uses to drive the ROM tests, plus a GIF
    // capture along the way.
    let mut hold_until = |spec: &mut spectrum::Spectrum, ch: char, field: &str, target: u16, max_frames: u32| {
        let pos = spectrum::keyboard::key_for_char(ch).unwrap().0;
        spec.set_key(pos, true);
        for i in 0..max_frames {
            spec.run_frame();
            if i % 2 == 0 {
                frames.push(spec.screen_indexed());
            }
            if read_u16(spec, field) == target {
                break;
            }
        }
        spec.set_key(pos, false);
    };

    eprintln!("solving room 0: axe -> vine -> exit...");
    hold_until(&mut spec, 'q', "y", 5, 150);
    hold_until(&mut spec, 'p', "held", 1, 150);
    hold_until(&mut spec, 'p', "vine_cleared", 1, 150);
    hold_until(&mut spec, 'p', "x", 29, 150);
    hold_until(&mut spec, 'a', "room", 1, 150);

    eprintln!("solving room 1: key -> door -> treasure...");
    hold_until(&mut spec, 'q', "y", 5, 150);
    hold_until(&mut spec, 'p', "held", 2, 150);
    hold_until(&mut spec, 'p', "door_open", 1, 150);
    hold_until(&mut spec, 'p', "x", 29, 150);
    hold_until(&mut spec, 'a', "won", 1, 150);

    // Hold on the win screen a moment so the GIF doesn't cut off mid-transition.
    for i in 0..30 {
        spec.run_frame();
        if i % 2 == 0 {
            frames.push(spec.screen_indexed());
        }
    }

    let won = read_u16(&spec, "won") != 0;
    eprintln!(
        "quest solved: {won} (axe_taken={} vine_cleared={} key_taken={} door_open={})",
        read_u16(&spec, "axe_taken") != 0,
        read_u16(&spec, "vine_cleared") != 0,
        read_u16(&spec, "key_taken") != 0,
        read_u16(&spec, "door_open") != 0,
    );

    let gif = display::gif::encode_indexed_to_vec(&frames, 256, 192, &display::AUTHENTIC, 4);
    if let Err(e) = std::fs::write(&out, &gif) {
        eprintln!("error: cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "wrote {out} ({} bytes, {} frames, quest solved: {won})",
        gif.len(),
        frames.len()
    );
    if won {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

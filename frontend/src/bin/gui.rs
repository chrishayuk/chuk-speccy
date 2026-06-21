//! Native pixel-perfect window head — the desktop head from
//! `docs/05-frontends-display-spec.md`. Renders the true 256×192 Spectrum display
//! (plus border) through the shared `display` pipeline into an integer-scaled
//! window, with real key-up/down events driving the keyboard matrix. No terminal
//! resolution ceiling; this is the crisp way to play.
//!
//! Usage: `speccy-gui <48.rom> [game.tap|.sna|.z80 | "game title"] [theme] [scaleN] [fullscreen]`
//!   game:   a local `.tap`/`.sna`/`.z80` file, OR a title to fetch from World of
//!           Spectrum (e.g. `speccy-gui 48.rom "Jet Set Willy"`).
//!   theme:  authentic | dark | light | terminal | amber | gameboy  (default authentic)
//!   scaleN: integer pixel zoom, e.g. `scale3` (default: auto-fit a sensible size)
//!   fullscreen: borderless, screen-filling, on top — for a second display / AirPlay.
//!   audiodev=NAME: send sound to the output whose name contains NAME.
//!   audiolist: print the available output audio devices and exit.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use display::{BorderMode, DisplayConfig};
use minifb::{Key, Scale, Window, WindowOptions};
use spectrum::keyboard::{self, KeyPos};
use spectrum::Spectrum;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: speccy-gui <48.rom> [game.tap|.sna|.z80 | \"title\"] [theme] [scaleN] [fullscreen]");
            std::process::exit(2);
        }
    };

    let mut theme_name = "authentic".to_string();
    let mut media_path: Option<String> = None;
    let mut scale = Scale::FitScreen;
    let mut fullscreen = false;
    let mut audio_device: Option<String> = None;
    let mut query_parts: Vec<String> = Vec::new();
    for a in args {
        if a == "audiolist" {
            list_output_devices();
            return;
        } else if let Some(name) = a.strip_prefix("audiodev=") {
            // Route the beeper to a named output (substring match), e.g. an
            // AirPlay/TV device when projecting. `audiolist` prints the choices.
            audio_device = Some(name.to_string());
        } else if a.ends_with(".sna") || a.ends_with(".z80") || a.ends_with(".tap") {
            media_path = Some(a);
        } else if a == "fullscreen" || a == "present" {
            // Borderless, screen-filling, always-on-top — clean for projecting to
            // a second display or over AirPlay screen mirroring.
            fullscreen = true;
        } else if let Some(n) = a.strip_prefix("scale") {
            scale = match n {
                "1" => Scale::X1,
                "2" => Scale::X2,
                "4" => Scale::X4,
                "8" => Scale::X8,
                _ => Scale::X2,
            };
        } else if DisplayConfig::preset(&a).is_some() {
            theme_name = a;
        } else {
            // Anything left over is part of a game title to fetch (a bare,
            // unquoted title arrives as several args — collect them all).
            query_parts.push(a);
        }
    }

    let mut cfg = DisplayConfig::preset(&theme_name).unwrap();
    cfg.border = BorderMode::Full;

    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("could not read ROM {rom_path}: {e}");
        std::process::exit(1);
    });

    let mut spec = Spectrum::new_48k(&rom);
    if let Some(p) = &media_path {
        let data = std::fs::read(p).unwrap_or_else(|e| {
            eprintln!("could not read {p}: {e}");
            std::process::exit(1);
        });
        let fmt = if p.ends_with(".tap") {
            "tap"
        } else if p.ends_with(".sna") {
            "sna"
        } else {
            "z80"
        };
        load_media(&mut spec, fmt, &data);
    } else if !query_parts.is_empty() {
        // A title was named: search World of Spectrum, download, and load it.
        let query = query_parts.join(" ");
        eprintln!("searching World of Spectrum for {query:?}…");
        match wos::fetch(&query) {
            Ok(game) => {
                let year = game.year.map(|y| format!(" ({y})")).unwrap_or_default();
                eprintln!("loaded {}{} [{}]", game.title, year, game.format);
                load_media(&mut spec, &game.format, &game.data);
            }
            Err(e) => {
                eprintln!("could not fetch {query:?}: {e}");
                std::process::exit(1);
            }
        }
    } else {
        for _ in 0..250 {
            spec.run_frame(); // boot to the prompt
        }
    }

    // Start audio (best-effort; the game stays playable if it fails). The cpal
    // callback drains `ring`; the emulation loop refills it (audio-driven pacing).
    let ring: Audio = Arc::new(Mutex::new(VecDeque::new()));
    let mut audio_rate = 0u32;
    let _stream = match start_audio(ring.clone(), audio_device.as_deref()) {
        Ok((stream, rate)) => {
            spec.enable_audio(rate);
            audio_rate = rate;
            Some(stream)
        }
        Err(e) => {
            eprintln!("audio unavailable ({e}); running silent");
            None
        }
    };
    // Keep ~3 frames of audio buffered; the device consumes at real time, so
    // refilling to this level paces emulation to the audio clock (no underrun).
    let target_fill = (audio_rate as usize / 50) * 3;

    // Size the window from the first rendered frame.
    let probe = display::render(&spec.screen_indexed(), spec.border(), &cfg);
    let (w, h) = (probe.width, probe.height);
    // Fullscreen fills the screen borderless (aspect preserved, letterboxed) and
    // floats on top — drag it to the projected/AirPlay display.
    let scale = if fullscreen { Scale::FitScreen } else { scale };
    let mut window = Window::new(
        &format!("chuk-speccy — {theme_name}"),
        w,
        h,
        WindowOptions {
            scale,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: !fullscreen,
            borderless: fullscreen,
            topmost: fullscreen,
            ..WindowOptions::default()
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("could not open window: {e}");
        std::process::exit(1);
    });
    window.set_target_fps(50); // minifb paces update() to ~50 Hz

    let mut buf = vec![0u32; w * h];
    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Rebuild the keyboard matrix from the currently-held keys (real up/down).
        spec.clear_keys();
        for key in window.get_keys() {
            apply_key(&mut spec, key);
        }

        // Advance one frame for the video, then run extra frames while the audio
        // buffer is below target (so emulation keeps pace with real-time audio
        // consumption even if the video refresh jitters). Capped so a stall can't
        // spiral. With audio off, this is just one frame per refresh.
        let mut frames_this_tick = 0;
        loop {
            spec.run_frame();
            frames_this_tick += 1;
            push_audio(&ring, spec.drain_audio());
            let queued = ring.lock().map(|q| q.len()).unwrap_or(usize::MAX);
            if audio_rate == 0 || queued >= target_fill || frames_this_tick >= 6 {
                break;
            }
        }

        let frame = display::render(&spec.screen_indexed(), spec.border(), &cfg);
        for (dst, px) in buf.iter_mut().zip(frame.rgba.chunks_exact(4)) {
            *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        // The render size never changes for a fixed border, so w/h are stable.
        let _ = window.update_with_buffer(&buf, frame.width, frame.height);
    }
}

type Audio = Arc<Mutex<VecDeque<f32>>>;

/// Cap the queued mono samples to bound latency (~200 ms at 44.1 kHz).
const AUDIO_QUEUE_CAP: usize = 8820;

/// Append samples to the ring, dropping the oldest beyond the latency cap.
fn push_audio(ring: &Audio, samples: Vec<f32>) {
    if samples.is_empty() {
        return;
    }
    if let Ok(mut q) = ring.lock() {
        q.extend(samples);
        while q.len() > AUDIO_QUEUE_CAP {
            q.pop_front();
        }
    }
}

/// Load a game by format. `.tap` boots to the BASIC prompt, inserts the tape and
/// `LOAD ""`s it (the ROM trap fast-loads the blocks while the window runs);
/// snapshots (`.sna`/`.z80`) load directly into a running machine.
fn load_media(spec: &mut Spectrum, fmt: &str, data: &[u8]) {
    if fmt == "tap" {
        for _ in 0..250 {
            spec.run_frame();
        }
        if let Err(e) = spec.load_tap(data) {
            eprintln!("tape load failed: {e:?}");
        } else {
            spec.autoload_tape();
        }
    } else if let Err(e) = spec.load_snapshot(fmt, data) {
        eprintln!("snapshot load failed: {e:?}");
    }
}

/// Open an output device and start a stream that drains `ring`. `device_name`
/// selects an output by case-insensitive substring (else the default). Returns
/// the live stream (keep it alive) and the sample rate to feed the emulator.
fn start_audio(ring: Audio, device_name: Option<&str>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = pick_output_device(&host, device_name)?;
    if let Ok(name) = device.name() {
        eprintln!("audio device: {name}");
    }
    let config = device
        .default_output_config()
        .map_err(|e| e.to_string())?;
    // This stream uses an f32 callback; on macOS CoreAudio the default is f32.
    // If a platform's default isn't f32 the audio would be wrong, so surface it.
    if config.sample_format() != cpal::SampleFormat::F32 {
        eprintln!(
            "warning: output sample format is {:?}, not F32 — audio may be wrong",
            config.sample_format()
        );
    }
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    eprintln!("audio: {rate} Hz, {channels} ch, {:?}", config.sample_format());
    let cfg: cpal::StreamConfig = config.into();

    let stream = device
        .build_output_stream(
            &cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut q = ring.lock().unwrap();
                for frame in data.chunks_mut(channels) {
                    let s = q.pop_front().unwrap_or(0.0);
                    for c in frame.iter_mut() {
                        *c = s; // same sample to every channel (mono beeper)
                    }
                }
            },
            move |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

/// Pick an output device by case-insensitive substring of its name, falling back
/// to the system default (with a warning) when nothing matches.
fn pick_output_device(host: &cpal::Host, want: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(substr) = want {
        let lc = substr.to_lowercase();
        if let Ok(devices) = host.output_devices() {
            for d in devices {
                if d.name().map(|n| n.to_lowercase().contains(&lc)).unwrap_or(false) {
                    return Ok(d);
                }
            }
        }
        eprintln!("no audio output matching {substr:?}; using the default");
    }
    host.default_output_device().ok_or_else(|| "no output device".to_string())
}

/// Print the available output devices (marking the default), then return.
fn list_output_devices() {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());
    println!("output audio devices:");
    match host.output_devices() {
        Ok(devices) => {
            for d in devices {
                if let Ok(name) = d.name() {
                    let mark = if Some(&name) == default.as_ref() { "  (default)" } else { "" };
                    println!("  {name}{mark}");
                }
            }
        }
        Err(e) => eprintln!("could not enumerate output devices: {e}"),
    }
}

/// Press the matrix key(s) for a held host key. Modifier keys map to CAPS/SYM
/// shift; letters/digits/space/enter map straight to their matrix position.
fn apply_key(spec: &mut Spectrum, key: Key) {
    let press = |spec: &mut Spectrum, pos: KeyPos| spec.set_key(pos, true);
    match key {
        Key::Enter => press(spec, keyboard::ENTER),
        Key::Space => press(spec, keyboard::SPACE),
        Key::LeftShift | Key::RightShift => press(spec, keyboard::CAPS_SHIFT),
        Key::LeftCtrl | Key::RightCtrl => press(spec, keyboard::SYM_SHIFT),
        Key::Backspace => {
            press(spec, keyboard::CAPS_SHIFT);
            press(spec, KeyPos { row: 4, col: 0 }); // DELETE = CAPS + 0
        }
        _ => {
            if let Some(ch) = key_char(key) {
                if let Some((pos, caps, sym)) = keyboard::key_for_char(ch) {
                    if caps {
                        press(spec, keyboard::CAPS_SHIFT);
                    }
                    if sym {
                        press(spec, keyboard::SYM_SHIFT);
                    }
                    press(spec, pos);
                }
            }
        }
    }
}

/// Map a minifb key to the character the matrix table understands.
fn key_char(key: Key) -> Option<char> {
    use Key::*;
    Some(match key {
        A => 'a', B => 'b', C => 'c', D => 'd', E => 'e', F => 'f', G => 'g',
        H => 'h', I => 'i', J => 'j', K => 'k', L => 'l', M => 'm', N => 'n',
        O => 'o', P => 'p', Q => 'q', R => 'r', S => 's', T => 't', U => 'u',
        V => 'v', W => 'w', X => 'x', Y => 'y', Z => 'z',
        Key0 => '0', Key1 => '1', Key2 => '2', Key3 => '3', Key4 => '4',
        Key5 => '5', Key6 => '6', Key7 => '7', Key8 => '8', Key9 => '9',
        _ => return None,
    })
}

//! Native pixel-perfect window head — the desktop head from
//! `docs/05-frontends-display-spec.md`. A **winit** window with a CPU framebuffer
//! (**softbuffer**) renders the true 256×192 Spectrum display (plus border)
//! through the shared `display` pipeline, aspect-correct and letterboxed, with
//! real key up/down driving the keyboard matrix and beeper audio via cpal.
//!
//! It's a real app shell: native **menus** (muda) — a *View* menu to toggle full
//! screen (plus the native macOS green button and F11), and an *Audio* menu to
//! switch the output device live (e.g. an AirPlay/TV speaker when projecting).
//!
//! Usage: `speccy-gui <48.rom> [game.tap|.sna|.z80 | "game title"] [theme] [scaleN] [fullscreen]`
//!   game:   a local `.tap`/`.sna`/`.z80` file, OR a title to fetch from World of
//!           Spectrum (e.g. `speccy-gui 48.rom "Jet Set Willy"`).
//!   theme:  authentic | dark | light | terminal | amber | gameboy  (default authentic)
//!   scaleN: integer pixel zoom, e.g. `scale3` (default 3). Ignored in fullscreen.
//!   fullscreen: start full screen (borderless). Toggle anytime via View menu / F11.
//!   audiodev=NAME: start with sound on the output whose name contains NAME.
//!   audiolist: print the available output audio devices and exit.

use std::collections::{HashSet, VecDeque};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use display::DisplayConfig;
use muda::{CheckMenuItem, Menu, MenuEvent, PredefinedMenuItem, Submenu};
use softbuffer::{Context, Surface};
use spectrum::keyboard::{self, KeyPos};
use spectrum::Spectrum;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

type Audio = Arc<Mutex<VecDeque<f32>>>;
type Surf = Surface<Rc<Window>, Rc<Window>>;

/// Cap the queued mono samples to bound latency (~200 ms at 44.1 kHz).
const AUDIO_QUEUE_CAP: usize = 8820;
/// One emulated 50 Hz frame; the redraw timer paces to this.
const FRAME_DT: Duration = Duration::from_micros(20_000);

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
    let mut init_scale: u32 = 3;
    let mut start_fullscreen = false;
    let mut audio_device: Option<String> = None;
    let mut query_parts: Vec<String> = Vec::new();
    for a in args {
        if a == "audiolist" {
            list_output_devices();
            return;
        } else if let Some(name) = a.strip_prefix("audiodev=") {
            audio_device = Some(name.to_string());
        } else if a.ends_with(".sna") || a.ends_with(".z80") || a.ends_with(".tap") {
            media_path = Some(a);
        } else if a == "fullscreen" || a == "present" {
            start_fullscreen = true;
        } else if let Some(n) = a.strip_prefix("scale") {
            init_scale = n.parse().unwrap_or(3).clamp(1, 12);
        } else if DisplayConfig::preset(&a).is_some() {
            theme_name = a;
        } else {
            // Anything left over is part of a game title to fetch (a bare,
            // unquoted title arrives as several args — collect them all).
            query_parts.push(a);
        }
    }

    let mut cfg = DisplayConfig::preset(&theme_name).unwrap();
    cfg.border = display::BorderMode::Full;

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
        let fmt = if p.ends_with(".tap") { "tap" } else if p.ends_with(".sna") { "sna" } else { "z80" };
        load_media(&mut spec, fmt, &data);
    } else if !query_parts.is_empty() {
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

    // Audio (best-effort; the game stays playable if it fails).
    let ring: Audio = Arc::new(Mutex::new(VecDeque::new()));
    let (stream, audio_rate) = match start_audio(ring.clone(), audio_device.as_deref()) {
        Ok((s, rate)) => {
            spec.enable_audio(rate);
            (Some(s), rate)
        }
        Err(e) => {
            eprintln!("audio unavailable ({e}); running silent");
            (None, 0)
        }
    };

    let mut app = Gui {
        spec,
        cfg,
        title: format!("chuk-speccy — {theme_name}"),
        init_scale,
        start_fullscreen,
        ring,
        stream,
        audio_rate,
        target_fill: (audio_rate as usize / 50) * 3,
        audio_devices: output_device_names(),
        audio_current: resolve_device_name(audio_device.as_deref()),
        window: None,
        surface: None,
        held: HashSet::new(),
        next_frame: Instant::now(),
        audio_items: Vec::new(),
        _menu: None,
        menu_built: false,
    };

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {e}");
    }
}

/// The app: emulator + window + audio + native menus, all on the main thread.
struct Gui {
    spec: Spectrum,
    cfg: DisplayConfig,
    title: String,
    init_scale: u32,
    start_fullscreen: bool,
    // audio
    ring: Audio,
    stream: Option<cpal::Stream>,
    audio_rate: u32,
    target_fill: usize,
    audio_devices: Vec<String>,
    audio_current: Option<String>,
    // window / render
    window: Option<Rc<Window>>,
    surface: Option<Surf>,
    held: HashSet<KeyCode>,
    next_frame: Instant,
    // menus (muda) — kept alive for the program's life
    audio_items: Vec<CheckMenuItem>,
    _menu: Option<Menu>,
    menu_built: bool,
}

impl ApplicationHandler for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // already created (e.g. app un-suspended)
        }
        let probe = display::render(&self.spec.screen_indexed(), self.spec.border(), &self.cfg);
        let (fw, fh) = (probe.width as f64, probe.height as f64);
        let s = self.init_scale as f64;
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(fw * s, fh * s));
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        if self.start_fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");
        self.window = Some(window);
        self.surface = Some(surface);
        self.next_frame = Instant::now();
        self.build_menu();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                match event.state {
                    ElementState::Pressed => {
                        // App chrome (not Spectrum keys): don't feed to the matrix.
                        match code {
                            KeyCode::F11 => self.toggle_fullscreen(),
                            KeyCode::Escape => self.exit_fullscreen(),
                            _ => {
                                self.held.insert(code);
                            }
                        }
                    }
                    ElementState::Released => {
                        self.held.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.run_frame_tick();
                self.present();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_menu();
        let now = Instant::now();
        if now >= self.next_frame {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            self.next_frame += FRAME_DT;
            if self.next_frame < now {
                self.next_frame = now + FRAME_DT; // we fell behind; don't spiral
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

impl Gui {
    /// Advance the emulation one frame, topping up extra frames while the audio
    /// buffer is below target so emulation tracks the real-time audio clock.
    fn run_frame_tick(&mut self) {
        self.spec.clear_keys();
        for &code in &self.held {
            apply_keycode(&mut self.spec, code);
        }
        let mut n = 0;
        loop {
            self.spec.run_frame();
            n += 1;
            push_audio(&self.ring, self.spec.drain_audio());
            let queued = self.ring.lock().map(|q| q.len()).unwrap_or(usize::MAX);
            if self.audio_rate == 0 || queued >= self.target_fill || n >= 6 {
                break;
            }
        }
    }

    /// Render the current screen and blit it (aspect-correct, letterboxed) into
    /// the window's framebuffer.
    fn present(&mut self) {
        let frame = display::render(&self.spec.screen_indexed(), self.spec.border(), &self.cfg);
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let (Some(pw), Some(ph)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return; // minimised
        };
        if surface.resize(pw, ph).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        scale_blit(&frame.rgba, frame.width, frame.height, &mut buf, pw.get() as usize, ph.get() as usize);
        let _ = buf.present();
    }

    // --- native menus -------------------------------------------------------

    fn build_menu(&mut self) {
        if self.menu_built {
            return;
        }
        let menu = Menu::new();
        // macOS app menu (gives Cmd+Q); harmless label elsewhere.
        let app = Submenu::new("speccy", true);
        let _ = app.append(&PredefinedMenuItem::quit(None));
        let _ = menu.append(&app);

        // A "View" menu that macOS auto-populates with its native "Enter Full
        // Screen" (⌃⌘F) + window-tabbing items — so we add no fullscreen item of
        // our own (that produced a duplicate). The green button and F11 also work.
        let view = Submenu::new("View", true);
        let _ = menu.append(&view);

        let audio = Submenu::new("Audio", true);
        let mut items = Vec::new();
        for name in &self.audio_devices {
            let checked = self.audio_current.as_deref() == Some(name.as_str());
            let it = CheckMenuItem::new(name, true, checked, None);
            let _ = audio.append(&it);
            items.push(it);
        }
        let _ = menu.append(&audio);

        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();

        self.audio_items = items;
        self._menu = Some(menu);
        self.menu_built = true;
    }

    fn poll_menu(&mut self) {
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if let Some(idx) = self.audio_items.iter().position(|it| it.id() == &ev.id) {
                let name = self.audio_devices[idx].clone();
                self.switch_audio(&name);
            }
        }
    }

    // --- fullscreen ---------------------------------------------------------

    fn toggle_fullscreen(&mut self) {
        if let Some(w) = &self.window {
            let on = w.fullscreen().is_some();
            w.set_fullscreen(if on { None } else { Some(Fullscreen::Borderless(None)) });
        }
    }

    fn exit_fullscreen(&mut self) {
        if let Some(w) = &self.window {
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
            }
        }
    }

    // --- audio device switching --------------------------------------------

    fn switch_audio(&mut self, name: &str) {
        self.stream = None; // drop the old stream first (release the device)
        match start_audio(self.ring.clone(), Some(name)) {
            Ok((stream, rate)) => {
                if rate != self.audio_rate {
                    self.spec.enable_audio(rate);
                    self.audio_rate = rate;
                    self.target_fill = (rate as usize / 50) * 3;
                }
                self.spec.drain_audio();
                self.stream = Some(stream);
                self.audio_current = Some(name.to_string());
                for (i, it) in self.audio_items.iter().enumerate() {
                    it.set_checked(self.audio_devices[i] == name);
                }
            }
            Err(e) => eprintln!("audio switch to {name:?} failed: {e}"),
        }
    }
}

/// Nearest-neighbour scale `src` (RGBA, `sw`×`sh`) into `dst` (0RGB u32, `dw`×`dh`),
/// preserving aspect ratio and centring with a black letterbox.
fn scale_blit(src: &[u8], sw: usize, sh: usize, dst: &mut [u32], dw: usize, dh: usize) {
    for p in dst.iter_mut() {
        *p = 0;
    }
    if sw == 0 || sh == 0 {
        return;
    }
    let scale = (dw as f64 / sw as f64).min(dh as f64 / sh as f64);
    let (ow, oh) = ((sw as f64 * scale) as usize, (sh as f64 * scale) as usize);
    if ow == 0 || oh == 0 {
        return;
    }
    let (ox, oy) = ((dw - ow) / 2, (dh - oh) / 2);
    for y in 0..oh {
        let sy = y * sh / oh;
        let drow = (oy + y) * dw + ox;
        let srow = sy * sw;
        for x in 0..ow {
            let s = (srow + x * sw / ow) * 4;
            dst[drow + x] = (src[s] as u32) << 16 | (src[s + 1] as u32) << 8 | src[s + 2] as u32;
        }
    }
}

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

/// Load a game by format. `.tap` boots to the prompt and `LOAD ""`s it via the
/// ROM trap (instant); `.tzx` `LOAD ""`s it and plays the tape *signal* in real
/// time (turbo/custom loaders — watch it load); snapshots load directly.
fn load_media(spec: &mut Spectrum, fmt: &str, data: &[u8]) {
    match fmt {
        "tap" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            if let Err(e) = spec.load_tap(data) {
                eprintln!("tape load failed: {e:?}");
            } else {
                spec.autoload_tape();
            }
        }
        "tzx" => {
            for _ in 0..250 {
                spec.run_frame();
            }
            spec.autoload_tape(); // type LOAD "" — the loader reads the signal
            if let Err(e) = spec.play_tape("tzx", data) {
                eprintln!("tape load failed: {e:?}");
            } else {
                eprintln!("loading from tape in real time…");
            }
        }
        _ => {
            if let Err(e) = spec.load_snapshot(fmt, data) {
                eprintln!("snapshot load failed: {e:?}");
            }
        }
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
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        eprintln!("warning: output sample format is {:?}, not F32 — audio may be wrong", config.sample_format());
    }
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
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

/// Names of the available output devices (for the Audio menu / `audiolist`).
fn output_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// The name `pick_output_device` would resolve `want` to (for the menu's tick).
fn resolve_device_name(want: Option<&str>) -> Option<String> {
    let host = cpal::default_host();
    pick_output_device(&host, want).ok().and_then(|d| d.name().ok())
}

/// Print the available output devices (marking the default), then return.
fn list_output_devices() {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());
    println!("output audio devices:");
    for name in output_device_names() {
        let mark = if Some(&name) == default.as_ref() { "  (default)" } else { "" };
        println!("  {name}{mark}");
    }
}

/// Map a winit physical key into the matrix key(s). Modifiers map to CAPS/SYM
/// shift; letters/digits/space/enter map straight to their matrix position.
fn apply_keycode(spec: &mut Spectrum, code: KeyCode) {
    let press = |spec: &mut Spectrum, pos: KeyPos| spec.set_key(pos, true);
    match code {
        KeyCode::Enter | KeyCode::NumpadEnter => press(spec, keyboard::ENTER),
        KeyCode::Space => press(spec, keyboard::SPACE),
        KeyCode::ShiftLeft | KeyCode::ShiftRight => press(spec, keyboard::CAPS_SHIFT),
        KeyCode::ControlLeft | KeyCode::ControlRight => press(spec, keyboard::SYM_SHIFT),
        KeyCode::Backspace => {
            press(spec, keyboard::CAPS_SHIFT);
            press(spec, KeyPos { row: 4, col: 0 }); // DELETE = CAPS + 0
        }
        _ => {
            if let Some(ch) = keycode_char(code) {
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

/// Map a winit `KeyCode` to the character the matrix table understands.
fn keycode_char(code: KeyCode) -> Option<char> {
    use KeyCode::*;
    Some(match code {
        KeyA => 'a', KeyB => 'b', KeyC => 'c', KeyD => 'd', KeyE => 'e', KeyF => 'f',
        KeyG => 'g', KeyH => 'h', KeyI => 'i', KeyJ => 'j', KeyK => 'k', KeyL => 'l',
        KeyM => 'm', KeyN => 'n', KeyO => 'o', KeyP => 'p', KeyQ => 'q', KeyR => 'r',
        KeyS => 's', KeyT => 't', KeyU => 'u', KeyV => 'v', KeyW => 'w', KeyX => 'x',
        KeyY => 'y', KeyZ => 'z',
        Digit0 => '0', Digit1 => '1', Digit2 => '2', Digit3 => '3', Digit4 => '4',
        Digit5 => '5', Digit6 => '6', Digit7 => '7', Digit8 => '8', Digit9 => '9',
        _ => return None,
    })
}

//! Diagnose beeper pitch: load a game, capture title audio, and report the
//! dominant frequency per 60 ms window (via zero-crossings). Compares contention
//! on vs off, since CPU timing sets beeper-loop pitch.
//! `cargo run -p spectrum --example audiodiag -- testroms/48.rom testroms/manic.z80`

use spectrum::Spectrum;

const RATE: u32 = 44_100;

fn capture(rom: &[u8], game: &[u8], contention: bool, frames: usize) -> Vec<f32> {
    let mut spec = Spectrum::new_48k(rom);
    spec.board.ula.contention_enabled = contention;
    let fmt = "z80";
    spec.load_snapshot(fmt, game).unwrap();
    spec.enable_audio(RATE);
    let mut all = Vec::new();
    for _ in 0..frames {
        spec.run_frame();
        all.extend(spec.drain_audio());
    }
    all
}

/// Dominant frequency per window via zero-crossing count (Hz).
fn pitches(samples: &[f32], window_ms: usize) -> Vec<u32> {
    let w = RATE as usize * window_ms / 1000;
    let mut out = Vec::new();
    for chunk in samples.chunks(w) {
        let mut crossings = 0;
        for pair in chunk.windows(2) {
            if (pair[0] <= 0.0) != (pair[1] <= 0.0) {
                crossings += 1;
            }
        }
        let secs = chunk.len() as f64 / RATE as f64;
        let hz = (crossings as f64 / 2.0 / secs) as u32;
        out.push(hz);
    }
    out
}

fn note_name(hz: u32) -> String {
    if hz < 20 {
        return "—".into();
    }
    // MIDI note from frequency; A4=440.
    let n = (12.0 * ((hz as f64 / 440.0).log2()) + 69.0).round() as i32;
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    if !(0..128).contains(&n) {
        return format!("{hz}Hz");
    }
    format!("{}{}", NAMES[(n % 12) as usize], n / 12 - 1)
}

fn report(label: &str, samples: &[f32]) {
    println!("\n== {label} ==  ({} samples, {:.2}s)", samples.len(), samples.len() as f64 / RATE as f64);
    let max = samples.iter().cloned().fold(f32::MIN, f32::max);
    let min = samples.iter().cloned().fold(f32::MAX, f32::min);
    println!("amplitude: {min:.3}..{max:.3}");
    let p = pitches(samples, 60);
    // Print the pitch sequence, skipping silence, collapsing repeats.
    let mut seq: Vec<u32> = Vec::new();
    for &hz in &p {
        if hz > 30 && seq.last() != Some(&hz) {
            seq.push(hz);
        }
    }
    print!("melody (first 24 distinct pitches): ");
    for &hz in seq.iter().take(24) {
        print!("{} ", note_name(hz));
    }
    println!();
    print!("              (Hz):                 ");
    for &hz in seq.iter().take(24) {
        print!("{hz} ");
    }
    println!();
}

fn main() {
    let rom = std::fs::read(std::env::args().nth(1).expect("rom")).unwrap();
    let game = std::fs::read(std::env::args().nth(2).expect("game")).unwrap();
    // ~3 seconds of title music.
    let on = capture(&rom, &game, true, 150);
    let off = capture(&rom, &game, false, 150);
    report("contention ON (current)", &on);
    report("contention OFF", &off);
}

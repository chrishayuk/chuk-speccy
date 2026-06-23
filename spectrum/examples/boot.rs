//! Boot the 48K ROM headless and print the decoded text screen.
//! Usage: `cargo run --release -p spectrum --example boot -- testroms/48.rom [frames]`

use spectrum::Spectrum;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: boot <48.rom> [frames]");
    let frames: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let rom = std::fs::read(&path).expect("read ROM");

    let mut spec = Spectrum::new_48k(&rom);
    for _ in 0..frames {
        spec.run_frame();
    }

    println!("+{}+", "-".repeat(32));
    for line in spec.screen_text().lines() {
        println!("|{line}|");
    }
    println!("+{}+", "-".repeat(32));

    let r = &spec.cpu.regs;
    eprintln!(
        "after {frames} frames: PC={:#06x} SP={:#06x} IM={} iff1={} frame={}",
        r.pc, r.sp, spec.cpu.im, spec.cpu.iff1, spec.board.ula.frame
    );
}

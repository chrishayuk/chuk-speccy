//! The Spectrum-native chatbot, end to end — a tiny **Z80 terminal** talking to
//! the host over the `ED FE` trap ABI (`docs/04-spectrum-native-chat-spec.md`).
//!
//! The Z80 program sends a canned line via `CHAT_BEGIN`, then drains the reply
//! with `CHAT_POLL`, printing each text event through the ROM print routine
//! (`RST $10`). The host side is `spectrum::host::chat_traps()` (echo responder).
//! No assembler needed — the ~55-byte program is hand-assembled below (and
//! disassembled at startup as a self-check).
//!
//! `cargo run -p spectrum --example chat_terminal -- testroms/48.rom`

use spectrum::Spectrum;

fn main() {
    let rom = std::fs::read(
        std::env::args()
            .nth(1)
            .expect("usage: chat_terminal <48.rom>"),
    )
    .unwrap();
    let mut spec = Spectrum::new_48k(&rom);

    // The Z80 terminal program (origin 0x8000):
    //   LD A,2 ; CALL $1601        ; open the upper-screen channel
    //   LD HL,prompt ; LD B,5 ; LD A,$30 ; HOSTCALL    ; CHAT_BEGIN
    // poll:
    //   LD HL,buf ; LD B,32 ; LD A,$31 ; HOSTCALL      ; CHAT_POLL -> A=ev, BC=len
    //   CP 2 ; JR Z,done                               ; DONE?
    //   CP 1 ; JR NZ,poll                              ; NONE -> poll again
    //   LD HL,buf
    // print:
    //   LD A,C ; OR A ; JR Z,poll                      ; printed all? next event
    //   PUSH HL ; PUSH BC ; LD A,(HL) ; RST $10 ; POP BC ; POP HL  ; print, regs preserved
    //   INC HL ; DEC C ; JR print
    // done: JR done
    // prompt: "HELLO"
    #[rustfmt::skip]
    let prog: [u8; 55] = [
        0x3E, 0x02,             // LD A,2
        0xCD, 0x01, 0x16,       // CALL $1601 (CHAN-OPEN)
        0x21, 0x32, 0x80,       // LD HL,$8032 (prompt)
        0x06, 0x05,             // LD B,5
        0x3E, 0x30,             // LD A,$30 (CHAT_BEGIN)
        0xED, 0xFE,             // HOSTCALL
        0x21, 0x37, 0x80,       // poll: LD HL,$8037 (buf)
        0x06, 0x20,             // LD B,32
        0x3E, 0x31,             // LD A,$31 (CHAT_POLL)
        0xED, 0xFE,             // HOSTCALL
        0xFE, 0x02,             // CP 2
        0x28, 0x15,             // JR Z,done
        0xFE, 0x01,             // CP 1
        0x20, 0xEF,             // JR NZ,poll
        0x21, 0x37, 0x80,       // LD HL,$8037 (buf)
        0x79,                   // print: LD A,C
        0xB7,                   // OR A
        0x28, 0xE8,             // JR Z,poll
        0xE5,                   // PUSH HL
        0xC5,                   // PUSH BC
        0x7E,                   // LD A,(HL)
        0xD7,                   // RST $10
        0xC1,                   // POP BC
        0xE1,                   // POP HL
        0x23,                   // INC HL
        0x0D,                   // DEC C
        0x18, 0xF2,             // JR print
        0x18, 0xFE,             // done: JR done
        0x48, 0x45, 0x4C, 0x4C, 0x4F, // "HELLO"
    ];

    // Boot to BASIC, then install the chat host + load and run the terminal.
    for _ in 0..250 {
        spec.run_frame();
    }
    spec.set_host_dispatcher(Box::new(spectrum::host::chat_traps()));
    spec.write_memory(0x8000, &prog);

    println!("Z80 terminal program (disassembled):");
    for line in spec.disassemble(0x8000, 24) {
        let bytes: String = line.bytes.iter().map(|b| format!("{b:02X} ")).collect();
        println!(
            "  {:04X}  {:<12} {}",
            line.addr,
            bytes.trim_end(),
            line.text
        );
        if line.text == "JR $8030" {
            break; // stop at the spin (rest is the "HELLO" data)
        }
    }

    spec.cpu.regs.pc = 0x8000;
    for _ in 0..4 {
        spec.run_frame();
    }

    println!("\nScreen after the chat round-trip:");
    for row in spec.screen_text().lines() {
        if !row.trim().is_empty() {
            println!("  {row}");
        }
    }
}

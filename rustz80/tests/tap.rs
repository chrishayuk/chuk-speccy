//! `.tap` emitter tests: the block structure (offline) and a full boot on the
//! real 48K ROM (gated behind `SPECTRUM_ROM`).

/// Split a `.tap` into its blocks' inner data (flag + payload, checksum stripped).
fn blocks(tap: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= tap.len() {
        let len = u16::from_le_bytes([tap[i], tap[i + 1]]) as usize;
        let block = &tap[i + 2..i + 2 + len];
        // Verify the XOR checksum (last byte) over the rest.
        let sum = block[..block.len() - 1].iter().fold(0u8, |a, &b| a ^ b);
        assert_eq!(sum, block[block.len() - 1], "bad checksum");
        out.push(block[..block.len() - 1].to_vec()); // flag + data, no checksum
        i += 2 + len;
    }
    assert_eq!(i, tap.len(), "trailing bytes");
    out
}

#[test]
fn tap_structure() {
    let code = [0x21, 0x2A, 0x00, 0xC9]; // LD HL,42 ; RET
    let tap = rustz80::to_tap(&code, 0x8000, 0x8000, "DEMO");
    let b = blocks(&tap);
    assert_eq!(b.len(), 4, "BASIC header+data, CODE header+data");

    // BASIC header.
    assert_eq!(b[0][0], 0x00, "header block flag");
    assert_eq!(b[0][1], 0, "type 0 = BASIC program");
    assert_eq!(&b[0][2..12], b"DEMO      ", "10-char name");
    assert_eq!(u16::from_le_bytes([b[0][14], b[0][15]]), 10, "autostart line 10");

    // BASIC data: line number 10 (big-endian) and a terminating ENTER.
    assert_eq!(b[1][0], 0xFF, "data block flag");
    assert_eq!(&b[1][1..3], &[0x00, 0x0A], "line number 10");
    assert_eq!(*b[1].last().unwrap(), 0x0D, "line ends with ENTER");

    // CODE header: type 3, load address 0x8000, length 4.
    assert_eq!(b[2][1], 3, "type 3 = CODE");
    assert_eq!(u16::from_le_bytes([b[2][12], b[2][13]]), 4, "code length");
    assert_eq!(u16::from_le_bytes([b[2][14], b[2][15]]), 0x8000, "load address");

    // CODE data == our bytes.
    assert_eq!(b[3][0], 0xFF, "data block flag");
    assert_eq!(&b[3][1..], &code, "code bytes round-trip");
}

#[test]
fn compile_to_tap_needs_entry() {
    assert!(rustz80::compile_to_tap("fn other() {}", "main", "X").is_err());
    assert!(rustz80::compile_to_tap("fn main() {}", "main", "X").is_ok());
}

/// Boot a dialect program from tape on the real ROM and confirm it executed.
///   SPECTRUM_ROM="$PWD/testroms/48.rom" cargo test -p rustz80 --test tap -- --ignored
#[test]
#[ignore = "set SPECTRUM_ROM to an absolute path to 48.rom"]
fn boots_on_real_rom() {
    let rom = std::fs::read(std::env::var("SPECTRUM_ROM").expect("SPECTRUM_ROM")).unwrap();

    // main(): write a 0xADDE sentinel above RAMTOP and set the top-left screen byte.
    let src = "
        fn main() {
            poke(40704u16, 222u8);   // 0x9F00 = 0xDE
            poke(40705u16, 173u8);   // 0x9F01 = 0xAD
            poke(16384u16, 255u8);   // top-left 8 pixels
        }
    ";
    let tap = rustz80::compile_to_tap(src, "main", "GAME").expect("compile");

    let mut spec = spectrum::Spectrum::new_48k(&rom);
    for _ in 0..250 {
        spec.run_frame(); // boot to the K cursor
    }
    spec.load_tap(&tap).unwrap();
    spec.autoload_tape(); // types LOAD ""
    for _ in 0..400 {
        spec.run_frame(); // trap-load BASIC + CODE, auto-run, USR main
    }

    let sentinel = spec.read_memory(0x9F00, 2);
    assert_eq!(sentinel, vec![222, 173], "main() ran from tape and wrote its sentinel");
    assert_eq!(spec.read_memory(0x4000, 1)[0], 0xFF, "top-left screen byte poked");
}

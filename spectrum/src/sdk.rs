//! Small ready-made Z80 programs that exercise the host-trap ABI — until the SDK
//! toolchain (assembler) lands, these are hand-assembled and verified by the
//! disassembler + tests.

/// Where [`CHAT_TERMINAL`] expects to be loaded and started.
pub const CHAT_TERMINAL_ORG: u16 = 0x8000;

/// An **interactive chat terminal** (`docs/04`): read a line from the keyboard
/// (echoed), and on ENTER send it to the host via `CHAT_BEGIN`, then teletype the
/// reply (in cyan) via `CHAT_POLL` + the ROM print routine. Loop forever. Drive
/// it with [`crate::host::chat_traps`] (or a Python dispatcher).
///
/// ```text
///         LD A,2 : CALL $1601              ; open the upper screen
///         LD HL,$5C3B : SET 3,(HL)         ; FLAGS bit 3 = L-mode (letters, not keywords)
/// new_line: XOR A : LD ($A0FE),A           ; input length = 0
/// wait_key: LD A,($5C3B) : BIT 5,A         ; FLAGS bit 5 = new key?
///         JR Z,wait_key
///         LD HL,$5C3B : RES 5,(HL)         ; consume it
///         LD A,($5C08)                     ; LAST-K
///         CP $0D : JP Z,send_line          ; ENTER → send
///         CP $20 : JR C,wait_key           ; ignore control / hi codes
///         CP $80 : JR NC,wait_key
///         LD B,A                           ; store char at buf+len, bump len
///         LD A,($A0FE) : LD L,A : LD H,0 : LD DE,$A000 : ADD HL,DE : LD (HL),B
///         LD A,($A0FE) : INC A : LD ($A0FE),A
///         LD A,B : RST $10                 ; echo
///         JP wait_key
/// send_line: LD A,$0D : RST $10            ; newline
///         LD A,($A0FE) : LD B,A : LD HL,$A000 : LD A,$30 : HOSTCALL   ; CHAT_BEGIN
///         LD A,$10 : RST $10 : LD A,5 : RST $10                        ; INK cyan
/// poll:   LD HL,$A100 : LD B,$20 : LD A,$31 : HOSTCALL                 ; CHAT_POLL
///         CP 2 : JR Z,reply_done
///         CP 1 : JR NZ,poll
///         LD HL,$A100
/// print:  LD A,C : OR A : JR Z,poll
///         PUSH HL : PUSH BC : LD A,(HL) : RST $10 : POP BC : POP HL : INC HL : DEC C : JR print
/// reply_done: LD A,$10 : RST $10 : LD A,0 : RST $10 : LD A,$0D : RST $10  ; INK black, newline
///         JP new_line
/// ```
#[rustfmt::skip]
pub const CHAT_TERMINAL: [u8; 132] = [
    0x3E, 0x02, 0xCD, 0x01, 0x16,                               // LD A,2 ; CALL $1601
    0x21, 0x3B, 0x5C, 0xCB, 0xDE,                               // LD HL,$5C3B ; SET 3,(HL) — L-mode
    0xAF, 0x32, 0xFE, 0xA0,                                     // new_line: XOR A ; LD ($A0FE),A
    0x3A, 0x3B, 0x5C, 0xCB, 0x6F, 0x28, 0xF9,                   // wait_key: LD A,($5C3B); BIT 5,A; JR Z,wait_key
    0x21, 0x3B, 0x5C, 0xCB, 0xAE, 0x3A, 0x08, 0x5C,            // LD HL,$5C3B; RES 5,(HL); LD A,($5C08)
    0xFE, 0x0D, 0xCA, 0x42, 0x80,                               // CP $0D ; JP Z,send_line
    0xFE, 0x20, 0x38, 0xE8,                                     // CP $20 ; JR C,wait_key
    0xFE, 0x80, 0x30, 0xE4,                                     // CP $80 ; JR NC,wait_key
    0x47, 0x3A, 0xFE, 0xA0, 0x6F, 0x26, 0x00, 0x11, 0x00, 0xA0, 0x19, 0x70, // store char at buf+len
    0x3A, 0xFE, 0xA0, 0x3C, 0x32, 0xFE, 0xA0,                   // len++
    0x78, 0xD7,                                                 // LD A,B ; RST $10 (echo)
    0xC3, 0x0E, 0x80,                                           // JP wait_key
    0x3E, 0x0D, 0xD7,                                           // send_line: LD A,$0D ; RST $10
    0x3A, 0xFE, 0xA0, 0x47, 0x21, 0x00, 0xA0, 0x3E, 0x30, 0xED, 0xFE, // CHAT_BEGIN(buf,len)
    0x3E, 0x10, 0xD7, 0x3E, 0x05, 0xD7,                         // INK cyan
    0x21, 0x00, 0xA1, 0x06, 0x20, 0x3E, 0x31, 0xED, 0xFE,      // poll: CHAT_POLL(buf,32)
    0xFE, 0x02, 0x28, 0x15,                                     // CP 2 ; JR Z,reply_done
    0xFE, 0x01, 0x20, 0xEF,                                     // CP 1 ; JR NZ,poll
    0x21, 0x00, 0xA1,                                           // LD HL,$A100
    0x79, 0xB7, 0x28, 0xE8,                                     // print: LD A,C ; OR A ; JR Z,poll
    0xE5, 0xC5, 0x7E, 0xD7, 0xC1, 0xE1, 0x23, 0x0D, 0x18, 0xF2, // print a char, preserve regs
    0x3E, 0x10, 0xD7, 0x3E, 0x00, 0xD7, 0x3E, 0x0D, 0xD7,      // reply_done: INK black, newline
    0xC3, 0x0A, 0x80,                                           // JP new_line
];

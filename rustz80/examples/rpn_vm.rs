//! A tiny **stack-machine bytecode VM** — a virtual machine compiled to run on a
//! virtual machine. Shows: arrays as a program + a stack, `match`-dispatched opcodes,
//! and a fetch–execute `while` loop. The program computes `6 * 7 + 5 = 47`.
//!
//! Dialect program: [`samples/showcase/rpn.rs`].
//!
//!     cargo run -p rustz80 --example rpn_vm

#[path = "common/cpu.rs"]
mod cpu;

const SRC: &str = include_str!("../samples/showcase/rpn.rs");

/// The oracle: the same RPN program evaluated in plain Rust.
fn host_eval() -> u16 {
    let prog = [0u16, 6, 0, 7, 3, 0, 5, 1];
    let mut stack = [0u16; 16];
    let (mut sp, mut pc) = (0usize, 0usize);
    while pc < prog.len() {
        match prog[pc] {
            0 => {
                pc += 1;
                stack[sp] = prog[pc];
                sp += 1;
            }
            1 => {
                let (b, a) = (stack[sp - 1], stack[sp - 2]);
                sp -= 1;
                stack[sp - 1] = a + b;
            }
            2 => {
                let (b, a) = (stack[sp - 1], stack[sp - 2]);
                sp -= 1;
                stack[sp - 1] = a - b;
            }
            _ => {
                let (b, a) = (stack[sp - 1], stack[sp - 2]);
                sp -= 1;
                stack[sp - 1] = a * b;
            }
        }
        pc += 1;
    }
    stack[0]
}

fn main() {
    let got = cpu::run_value(SRC, "eval", &[]);
    let want = host_eval();

    println!("RPN bytecode:  PUSH 6, PUSH 7, MUL, PUSH 5, ADD");
    println!("  result  z80 = {got}   rustc = {want}");
    assert_eq!(got, want, "z80 and rustc disagree");
    println!("  ✓ the VM computed 6*7 + 5 = 47 on the Z80");
}

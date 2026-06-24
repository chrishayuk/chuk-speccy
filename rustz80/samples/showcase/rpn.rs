// A tiny stack-machine bytecode VM, in the rustz80 dialect.
// Entry `eval` -> the result of running a fixed program: 6 * 7 + 5 = 47.
// Shows: arrays as program + stack, `match`-dispatched opcodes, a fetch loop.
//
// Opcodes: 0 = PUSH (operand follows), 1 = ADD, 2 = SUB, 3 = MUL.

fn eval() -> u16 {
    // PUSH 6, PUSH 7, MUL, PUSH 5, ADD   ==>   6*7 + 5
    let prog = [0u16, 6u16, 0u16, 7u16, 3u16, 0u16, 5u16, 1u16];
    let n = 8u16;

    let mut stack = [0u16; 16];
    let mut sp = 0u16;
    let mut pc = 0u16;

    while pc < n {
        let op = prog[pc as usize];
        match op {
            0u16 => {
                // PUSH: the next cell is the operand.
                pc = pc + 1u16;
                stack[sp as usize] = prog[pc as usize];
                sp = sp + 1u16;
            }
            1u16 => {
                let b = stack[(sp - 1u16) as usize];
                let a = stack[(sp - 2u16) as usize];
                sp = sp - 1u16;
                stack[(sp - 1u16) as usize] = a + b;
            }
            2u16 => {
                let b = stack[(sp - 1u16) as usize];
                let a = stack[(sp - 2u16) as usize];
                sp = sp - 1u16;
                stack[(sp - 1u16) as usize] = a - b;
            }
            _ => {
                let b = stack[(sp - 1u16) as usize];
                let a = stack[(sp - 2u16) as usize];
                sp = sp - 1u16;
                stack[(sp - 1u16) as usize] = a * b;
            }
        }
        pc = pc + 1u16;
    }
    stack[0]
}

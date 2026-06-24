// Const generics: a const parameter sizes a local array and bounds the loops, and
// each `::<N>` instantiates a specialized copy (`triangle$4`, `triangle$8`). The same
// source compiles under rustc and rustz80.
// Entry `run` -> triangle::<4>() * 100 + triangle::<8>().

fn triangle<const N: usize>() -> u16 {
    // The N-th triangular number, via a fixed [u16; N] scratch buffer.
    let mut a = [0u16; N];
    let mut i = 0usize;
    while i < N {
        a[i] = (i + 1) as u16;
        i = i + 1;
    }
    let mut s = 0u16;
    let mut j = 0usize;
    while j < N {
        s = s + a[j];
        j = j + 1;
    }
    s
}

fn run() -> u16 {
    triangle::<4>() * 100u16 + triangle::<8>() // 10*100 + 36 = 1036
}

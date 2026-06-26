//! The `Asm` assembler — emit primitives, label/call fixups, the local scratch layout.
use super::runtime::{DIVMOD16, MUL16};
use super::Target;
use std::collections::HashMap;

/// Locals: slot `i` lives at `SCRATCH + i*2` (`u16` each). Each function reuses
/// the same region (Stage 1 has no recursion / overlapping live ranges yet).
pub(super) const SCRATCH: u16 = 0x9000;

pub(super) struct Asm {
    pub(super) org: u16,
    pub(super) target: Target,
    pub(super) code: Vec<u8>,
    pub(super) labels: Vec<Option<u16>>,
    pub(super) label_fixups: Vec<(usize, usize)>,
    pub(super) symbols: HashMap<String, u16>,
    pub(super) call_fixups: Vec<(usize, String)>,
    pub(super) needs_mul: bool,
    pub(super) needs_div: bool,
    /// Slot offset for the function currently being emitted, so each function's
    /// locals occupy a disjoint scratch region (correct for non-recursive calls;
    /// real stack frames are a later stage).
    pub(super) base: u16,
    /// Enclosing loops as `(continue target, break target)` labels — the innermost
    /// is last. `continue`/`break` jump to the top entry's targets.
    pub(super) loop_stack: Vec<(usize, usize)>,
    /// The current function's epilogue label — `return` jumps here (the value is
    /// already in `HL`).
    pub(super) func_end: Option<usize>,
}

impl Asm {
    pub(super) fn new(org: u16, target: Target) -> Self {
        Asm {
            org,
            target,
            code: Vec::new(),
            labels: Vec::new(),
            label_fixups: Vec::new(),
            symbols: HashMap::new(),
            call_fixups: Vec::new(),
            needs_mul: false,
            needs_div: false,
            base: 0,
            loop_stack: Vec::new(),
            func_end: None,
        }
    }
    pub(super) fn here(&self) -> u16 {
        self.org.wrapping_add(self.code.len() as u16)
    }
    pub(super) fn byte(&mut self, b: u8) {
        self.code.push(b);
    }
    pub(super) fn word(&mut self, w: u16) {
        self.code.push(w as u8);
        self.code.push((w >> 8) as u8);
    }
    pub(super) fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    pub(super) fn place(&mut self, l: usize) {
        let here = self.here();
        self.labels[l] = Some(here);
    }
    pub(super) fn jump(&mut self, opcode: u8, l: usize) {
        self.byte(opcode);
        self.label_fixups.push((self.code.len(), l));
        self.word(0);
    }
    /// Emit `CALL name` (resolved to the symbol address at finish).
    pub(super) fn call(&mut self, name: &str) {
        self.byte(0xCD);
        self.call_fixups.push((self.code.len(), name.to_string()));
        self.word(0);
    }
    pub(super) fn define(&mut self, name: &str) {
        let here = self.here();
        self.symbols.insert(name.to_string(), here);
    }
    pub(super) fn finish(mut self) -> (Vec<u8>, HashMap<String, u16>) {
        // Append the micro-runtime routines that were used.
        if self.needs_mul {
            self.define("__mul16");
            self.code.extend_from_slice(MUL16);
        }
        if self.needs_div {
            self.define("__divmod16");
            self.code.extend_from_slice(DIVMOD16);
        }
        for (pos, l) in &self.label_fixups {
            let a = self.labels[*l].expect("unplaced label");
            self.code[*pos] = a as u8;
            self.code[*pos + 1] = (a >> 8) as u8;
        }
        for (pos, name) in &self.call_fixups {
            let a = *self
                .symbols
                .get(name)
                .unwrap_or_else(|| panic!("unknown call target {name}"));
            self.code[*pos] = a as u8;
            self.code[*pos + 1] = (a >> 8) as u8;
        }
        (self.code, self.symbols)
    }
}

pub(super) fn slot_addr(base: u16, slot: usize) -> u16 {
    SCRATCH + (base + slot as u16) * 2
}

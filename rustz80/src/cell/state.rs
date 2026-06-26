//! `StateCell` — typed I/O by field name (the JSON↔state agent surface).
use super::*;
use std::collections::HashMap;

/// Where a [`StateCell`]'s state struct is laid out — clear of code (`ORG`), the scratch
/// register file, the trampoline, and the stack.
pub const STATE_BASE: u16 = 0xB000;

/// A cell bound to a **state struct** at [`STATE_BASE`] — typed I/O by *field name*. The
/// agent/MCP surface for "named inputs in → run → named outputs out": compile once, then
/// `set` fields, `run`, `get` fields; the layout maps names to addresses. The program is a
/// method on the state (`impl State { fn run(&mut self) … }`), reached through `&mut self`.
///
/// ```
/// use rustz80::cell::{StateCell, DEFAULT_CYCLES};
/// let src = "struct State { x: u16, score: u16 }
///            impl State { fn run(&mut self) -> u16 { self.score = self.x * 2u16; self.score } }";
/// let mut cell = StateCell::bind(src, "State", None)?;
/// cell.set("x", 10)?;
/// cell.run(DEFAULT_CYCLES)?;
/// assert_eq!(cell.get("score"), Some(20));   // typed, by name — no raw addresses
/// # Ok::<(), String>(())
/// ```
pub struct StateCell {
    runner: Runner,
    addrs: HashMap<String, u16>, // scalar field name -> byte address
    entry: String,
    pending: Vec<(u16, Ty, u64)>,
}

impl StateCell {
    /// Compile `src`, bind its `state` struct's scalar fields at [`STATE_BASE`], and target
    /// `entry` (default `"<state>::run"`).
    pub fn bind(src: &str, state: &str, entry: Option<&str>) -> Result<Self, String> {
        let layout = crate::struct_layout(src, state)?;
        let mut addrs = HashMap::new();
        for f in &layout {
            if f.slots == 1 {
                // scalar field, addressable by name (a `u16`/`u8` slot)
                addrs.insert(f.name.clone(), STATE_BASE + f.offset * 2);
            }
        }
        Ok(StateCell {
            runner: Runner::compile(src)?,
            addrs,
            entry: entry.map_or_else(|| format!("{state}::run"), String::from),
            pending: Vec::new(),
        })
    }

    /// Queue a named `u16` input (written into the state before the next [`run`](StateCell::run)).
    pub fn set(&mut self, field: &str, value: u16) -> Result<(), String> {
        let &addr = self
            .addrs
            .get(field)
            .ok_or_else(|| format!("no scalar field `{field}`"))?;
        self.pending.push((addr, Ty::U16, value as u64));
        Ok(())
    }

    /// Run the entry with `&mut self` state at [`STATE_BASE`], applying then clearing the
    /// queued inputs.
    pub fn run(&mut self, budget: u64) -> Result<Report, String> {
        let pending = std::mem::take(&mut self.pending);
        self.runner
            .run_with_inputs(Some(&self.entry), &[STATE_BASE], &pending, budget)
    }

    /// Read a named `u16` field from the last run's state.
    pub fn get(&self, field: &str) -> Option<u16> {
        self.addrs.get(field).map(|&a| self.runner.peek_u16(a))
    }

    /// The bound (scalar) field names.
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.addrs.keys().map(String::as_str)
    }
}

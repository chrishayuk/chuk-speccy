//! Host-trap dispatch — the `ED FE` (`HOSTCALL`) ABI from
//! `docs/03-sdk-spec.md`. A Z80 program executes `ED FE` with a syscall id in
//! `A`; the CPU forwards it to [`z80::Bus::host_trap`], the [`Board`](crate::Board)
//! routes it to the installed [`HostCalls`] dispatcher, and the handler reads
//! args from the registers / memory and writes results back (carry = error).
//!
//! Two flavours of handler: [`FnTable`] (id → Rust closure, for math / asset DMA
//! / tests — never crosses a language boundary) and any custom [`HostCalls`] impl
//! (e.g. the PyO3 bridge that forwards to a Python callable).

use crate::memory::Memory;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use z80::Regs;

/// What a handler sees during a trap: the live register file plus scoped memory
/// access. Valid only for the duration of the synchronous dispatch call.
pub struct HostCtx<'a> {
    pub regs: &'a mut Regs,
    mem: &'a mut Memory,
}

impl<'a> HostCtx<'a> {
    pub(crate) fn new(regs: &'a mut Regs, mem: &'a mut Memory) -> Self {
        Self { regs, mem }
    }

    /// The syscall id (register `A`).
    #[inline]
    pub fn id(&self) -> u8 {
        self.regs.a
    }

    /// Read `len` bytes of the 64K space from `addr` (wrapping).
    pub fn read(&self, addr: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.mem.read(addr.wrapping_add(i))).collect()
    }

    /// Write `data` into memory at `addr` (ROM writes ignored, as on hardware).
    pub fn write(&mut self, addr: u16, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.mem.write(addr.wrapping_add(i as u16), b);
        }
    }

    /// Signal failure / success to the caller (carry flag).
    pub fn fail(&mut self) {
        self.regs.set_carry(true);
    }
    pub fn ok(&mut self) {
        self.regs.set_carry(false);
    }

    /// Raw pointers to the live register file and memory, for an FFI bridge that
    /// hands them to another language *during* the synchronous trap. The pointers
    /// are valid only for the duration of the dispatch call — the caller must not
    /// let them escape it (the PyO3 bridge enforces this with a liveness guard).
    #[doc(hidden)]
    pub fn raw_parts(&mut self) -> (*mut Regs, *mut Memory) {
        (&mut *self.regs as *mut Regs, &mut *self.mem as *mut Memory)
    }
}

/// Anything that answers host traps. `Send` so it can live behind threaded heads.
/// Return any extra T-states to charge for modelled latency.
pub trait HostCalls: Send {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32;
}

/// A single native trap handler.
type Handler = Box<dyn FnMut(&mut HostCtx) -> u32 + Send>;

/// A registry of id → Rust closure — the native handler path (math, asset DMA,
/// tests). An unknown id falls through to the optional `fallback` dispatcher
/// (e.g. a Python bridge), else fails cleanly (carry set, no-op). This is how
/// fast Rust math and a host-language chat handler compose into one dispatcher.
#[derive(Default)]
pub struct FnTable {
    map: HashMap<u8, Handler>,
    fallback: Option<Box<dyn HostCalls>>,
}

impl FnTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for syscall `id`.
    pub fn on(&mut self, id: u8, f: impl FnMut(&mut HostCtx) -> u32 + Send + 'static) {
        self.map.insert(id, Box::new(f));
    }

    /// Route ids this table doesn't handle to `next`.
    pub fn with_fallback(mut self, next: Box<dyn HostCalls>) -> Self {
        self.fallback = Some(next);
        self
    }
}

impl HostCalls for FnTable {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32 {
        match self.map.get_mut(&ctx.id()) {
            Some(h) => h(ctx),
            None => match self.fallback.as_mut() {
                Some(f) => f.dispatch(ctx),
                None => {
                    ctx.fail(); // unknown id → CF=1, no-op
                    0
                }
            },
        }
    }
}

/// The standard host math syscalls (id range `0x10–0x1F`) — the integer ops the
/// Z80 is slow at, done host-cheap. Install with
/// [`set_host_dispatcher`](crate::Spectrum::set_host_dispatcher).
pub fn math_traps() -> FnTable {
    let mut t = FnTable::new();
    // 0x10 MUL16: HL = BC * DE (wrapping).
    t.on(0x10, |c| {
        let (bc, de) = (c.regs.bc(), c.regs.de());
        c.regs.set_hl(bc.wrapping_mul(de));
        c.ok();
        0
    });
    // 0x11 DIVMOD16: HL = BC / DE, DE = BC % DE; carry set on divide-by-zero.
    t.on(0x11, |c| {
        let (bc, de) = (c.regs.bc(), c.regs.de());
        if de == 0 {
            c.fail();
            return 0;
        }
        c.regs.set_hl(bc / de);
        c.regs.set_de(bc % de);
        c.ok();
        0
    });
    t
}

// Chatbot trap ids (`docs/04-spectrum-native-chat-spec.md`); see also the
// Python `chat.py` host (the LLM-backed path).
pub const CHAT_BEGIN: u8 = 0x30;
pub const CHAT_POLL: u8 = 0x31;
pub const CHAT_CANCEL: u8 = 0x32;
pub const CHAT_RESET: u8 = 0x33;

// Reply event codes returned by CHAT_POLL in `A`.
const EV_NONE: u8 = 0;
const EV_TEXT: u8 = 1;
const EV_DONE: u8 = 2;

/// A reply turned into a queue of teletype events the Z80 drains via CHAT_POLL.
#[derive(Default)]
struct ChatState {
    history: Vec<String>,
    queue: VecDeque<(u8, Vec<u8>)>,
}

impl ChatState {
    fn begin(&mut self, prompt: &str) {
        self.history.push(prompt.to_string());
        // Stub responder — echoes the prompt. Swap in a real backend host-side
        // (the Python `chat.py` does chuk-llm).
        let reply = to_spectrum(&format!("You said: {prompt}"));
        for chunk in reply.chunks(16) {
            self.queue.push_back((EV_TEXT, chunk.to_vec()));
        }
        self.queue.push_back((EV_DONE, Vec::new()));
    }

    fn poll(&mut self) -> (u8, Vec<u8>) {
        self.queue.pop_front().unwrap_or((EV_NONE, Vec::new()))
    }
}

/// Clamp to the printable Spectrum charset (ASCII 32..126).
fn to_spectrum(s: &str) -> Vec<u8> {
    s.bytes().map(|b| if (32..=126).contains(&b) { b } else { b'?' }).collect()
}

/// The chatbot host syscalls (`CHAT_*`, id range `0x30–0x3F`) with a built-in
/// echo responder — the native counterpart of `chat.py`, so the Z80 terminal can
/// be driven without a Python layer. `CHAT_BEGIN`: `HL`→input, `B`=len.
/// `CHAT_POLL`: `HL`→buffer, `B`=cap → `A`=event (0 none/1 text/2 done), `BC`=len.
pub fn chat_traps() -> FnTable {
    let state = Arc::new(Mutex::new(ChatState::default()));
    let mut t = FnTable::new();

    let s = state.clone();
    t.on(CHAT_BEGIN, move |c| {
        let (addr, n) = (c.regs.hl(), c.regs.bc() >> 8);
        let prompt = String::from_utf8_lossy(&c.read(addr, n)).to_string();
        s.lock().unwrap().begin(&prompt);
        c.ok();
        0
    });

    let s = state.clone();
    t.on(CHAT_POLL, move |c| {
        let (addr, cap) = (c.regs.hl(), (c.regs.bc() >> 8) as usize);
        let (code, mut payload) = s.lock().unwrap().poll();
        payload.truncate(cap);
        c.write(addr, &payload);
        c.regs.a = code;
        c.regs.set_bc(payload.len() as u16);
        c.ok();
        0
    });

    let s = state.clone();
    t.on(CHAT_CANCEL, move |c| {
        s.lock().unwrap().queue.clear();
        c.ok();
        0
    });

    let s = state;
    t.on(CHAT_RESET, move |c| {
        let mut st = s.lock().unwrap();
        st.history.clear();
        st.queue.clear();
        c.ok();
        0
    });

    t
}

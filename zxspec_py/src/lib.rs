//! PyO3 bindings: expose the `spectrum` core to Python as a `Machine` class.
//! The Rust side stays format-agnostic — it returns raw RGBA / registers / bytes;
//! all MCP-shaping (PNG, base64, content blocks) happens in the Python layer
//! (`docs/02-mcp-server-layer-spec.md`).

mod trap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use pyo3::wrap_pyfunction;
use spectrum::keyboard::{self, KeyPos};
use spectrum::Spectrum;

/// A headless 48K Spectrum the host can step, observe, drive, and checkpoint.
#[pyclass]
struct Machine {
    spec: Spectrum,
    rom: Vec<u8>,
}

#[pymethods]
impl Machine {
    /// `Machine(rom: bytes, model="48k")` — `rom` is the 16K system ROM image.
    #[new]
    #[pyo3(signature = (rom, model="48k"))]
    fn new(rom: Vec<u8>, model: &str) -> PyResult<Self> {
        if model != "48k" {
            return Err(PyValueError::new_err(format!(
                "unsupported model {model:?} (only '48k')"
            )));
        }
        Ok(Self {
            spec: Spectrum::new_48k(&rom),
            rom,
        })
    }

    /// Power-cycle: rebuild from the original ROM.
    fn reset(&mut self) {
        self.spec = Spectrum::new_48k(&self.rom);
    }

    // --- state I/O -----------------------------------------------------------

    /// Load a snapshot (`fmt` = "sna" | "z80") or tape ("tap").
    fn load_snapshot(&mut self, fmt: &str, data: &[u8]) -> PyResult<()> {
        match fmt {
            "tap" => self
                .spec
                .load_tap(data)
                .map_err(|e| PyValueError::new_err(format!("{e:?}"))),
            _ => self
                .spec
                .load_snapshot(fmt, data)
                .map_err(|e| PyValueError::new_err(format!("{e:?}"))),
        }
    }

    /// Save the full machine state (currently `.sna`).
    fn save_snapshot<'py>(&self, py: Python<'py>, fmt: &str) -> PyResult<Bound<'py, PyBytes>> {
        match fmt {
            "sna" => Ok(PyBytes::new_bound(py, &self.spec.save_sna())),
            other => Err(PyValueError::new_err(format!("cannot save {other:?} yet"))),
        }
    }

    /// Insert a `.tap` and type `LOAD ""` to start the ROM fast-loader.
    fn autoload_tape(&mut self, data: &[u8]) -> PyResult<()> {
        self.spec
            .load_tap(data)
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))?;
        self.spec.autoload_tape();
        Ok(())
    }

    /// Type `LOAD ""` (without inserting a `.tap`) — for real-time tape loading
    /// where the signal, not the ROM trap, feeds the loader.
    fn type_load(&mut self) {
        self.spec.autoload_tape();
    }

    /// Start **real-time** tape playback from `.tap`/`.tzx` bytes (`fmt`). Drives
    /// the EAR line so turbo/custom loaders work; run frames to load it.
    fn play_tape(&mut self, fmt: &str, data: &[u8]) -> PyResult<()> {
        self.spec
            .play_tape(fmt, data)
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))
    }

    /// True while a real-time tape is still playing.
    fn tape_playing(&self) -> bool {
        self.spec.tape_playing()
    }

    // --- host traps (`ED FE`) -----------------------------------------------

    /// Install a Python callable `cb(ctx)` to answer `ED FE` host traps. `ctx`
    /// exposes the registers (`a`, `bc`, `hl`, … + setters), `carry`, and
    /// `read(addr,len)`/`write(addr,bytes)` — valid only during the call. Switch
    /// on `ctx.a` (the syscall id). Without one, `ED FE` is a NOP. With
    /// `with_math=True`, the native math syscalls (0x10–0x1F) are handled in Rust
    /// and only other ids reach `cb`.
    #[pyo3(signature = (cb, with_math = false))]
    fn register_host_dispatcher(&mut self, cb: Py<PyAny>, with_math: bool) {
        let py = Box::new(trap::PyDispatcher::new(cb));
        if with_math {
            self.spec.set_host_dispatcher(Box::new(spectrum::host::math_traps().with_fallback(py)));
        } else {
            self.spec.set_host_dispatcher(py);
        }
    }

    /// Install only the native math syscalls (0x10–0x1F) — no Python callback.
    fn install_math_traps(&mut self) {
        self.spec.set_host_dispatcher(Box::new(spectrum::host::math_traps()));
    }

    /// Remove the host dispatcher; `ED FE` reverts to a NOP.
    fn clear_host_dispatcher(&mut self) {
        self.spec.clear_host_dispatcher();
    }

    // --- execution -----------------------------------------------------------

    /// Execute `count` instructions.
    #[pyo3(signature = (count=1))]
    fn step(&mut self, count: u32) {
        for _ in 0..count {
            self.spec.step();
        }
    }

    /// Run `n` full frames (each ≈ 69888 T-states / one `/INT`).
    #[pyo3(signature = (n=1))]
    fn run_frames(&mut self, n: u32) {
        for _ in 0..n {
            self.spec.run_frame();
        }
    }

    /// Step until PC reaches `pc` (if given) or `max_steps` instructions elapse.
    /// Returns `{stop, pc, steps}` where stop is "pc" | "budget".
    #[pyo3(signature = (pc=None, max_steps=2_000_000))]
    fn run_until<'py>(
        &mut self,
        py: Python<'py>,
        pc: Option<u16>,
        max_steps: u64,
    ) -> Bound<'py, PyDict> {
        let mut steps = 0u64;
        let mut stop = "budget";
        while steps < max_steps {
            if Some(self.spec.cpu.regs.pc) == pc {
                stop = "pc";
                break;
            }
            self.spec.step();
            steps += 1;
        }
        if Some(self.spec.cpu.regs.pc) == pc {
            stop = "pc";
        }
        let d = PyDict::new_bound(py);
        d.set_item("stop", stop).unwrap();
        d.set_item("pc", self.spec.cpu.regs.pc).unwrap();
        d.set_item("steps", steps).unwrap();
        d
    }

    // --- observation ---------------------------------------------------------

    /// Full register state as a dict.
    fn registers<'py>(&self, py: Python<'py>) -> Bound<'py, PyDict> {
        let r = &self.spec.cpu.regs;
        let d = PyDict::new_bound(py);
        for (k, v) in [
            ("a", r.a as u32), ("f", r.f as u32),
            ("b", r.b as u32), ("c", r.c as u32),
            ("d", r.d as u32), ("e", r.e as u32),
            ("h", r.h as u32), ("l", r.l as u32),
            ("bc", r.bc() as u32), ("de", r.de() as u32),
            ("hl", r.hl() as u32), ("af", r.af() as u32),
            ("ix", r.ix as u32), ("iy", r.iy as u32),
            ("sp", r.sp as u32), ("pc", r.pc as u32),
            ("i", r.i as u32), ("r", r.r as u32),
            ("wz", r.wz as u32),
            ("im", self.spec.cpu.im as u32),
            ("iff1", self.spec.cpu.iff1 as u32),
            ("iff2", self.spec.cpu.iff2 as u32),
        ] {
            d.set_item(k, v).unwrap();
        }
        d
    }

    /// Read `len` bytes from `addr` (wrapping the 64K space).
    fn read_memory<'py>(&self, py: Python<'py>, addr: u16, len: u16) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.spec.read_memory(addr, len))
    }

    /// Disassemble `count` instructions from `addr`. Returns a list of
    /// `{addr, bytes, text}` dicts (the next instruction is at `addr + len(bytes)`).
    #[pyo3(signature = (addr, count=16))]
    fn disassemble<'py>(&self, py: Python<'py>, addr: u16, count: u16) -> Vec<Bound<'py, PyDict>> {
        self.spec
            .disassemble(addr, count)
            .into_iter()
            .map(|l| {
                let d = PyDict::new_bound(py);
                d.set_item("addr", l.addr).unwrap();
                d.set_item("bytes", PyBytes::new_bound(py, &l.bytes)).unwrap();
                d.set_item("text", l.text).unwrap();
                d
            })
            .collect()
    }

    /// Decoded display as RGBA bytes (256×192×4, no border).
    fn screen_rgba<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.spec.screen_rgba())
    }

    /// Display as logical colour indices (256×192, one byte/pixel).
    fn screen_indexed<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.spec.screen_indexed())
    }

    /// Current border colour (0–7).
    fn border(&self) -> u8 {
        self.spec.border()
    }

    /// Text-mode screen scrape (32×24, via the font in ROM).
    fn screen_text(&self) -> String {
        self.spec.screen_text()
    }

    // --- mutation / input ----------------------------------------------------

    /// Write `data` into memory at `addr` (ROM writes ignored).
    fn write_memory(&mut self, addr: u16, data: &[u8]) {
        self.spec.write_memory(addr, data);
    }

    /// Set a register by name (8- and 16-bit, incl. shadow `af'` via "af_").
    fn set_register(&mut self, name: &str, value: u32) -> PyResult<()> {
        let r = &mut self.spec.cpu.regs;
        let v8 = value as u8;
        let v16 = value as u16;
        match name {
            "a" => r.a = v8, "f" => r.f = v8,
            "b" => r.b = v8, "c" => r.c = v8,
            "d" => r.d = v8, "e" => r.e = v8,
            "h" => r.h = v8, "l" => r.l = v8,
            "i" => r.i = v8, "r" => r.r = v8,
            "bc" => r.set_bc(v16), "de" => r.set_de(v16),
            "hl" => r.set_hl(v16), "af" => r.set_af(v16),
            "ix" => r.ix = v16, "iy" => r.iy = v16,
            "sp" => r.sp = v16, "pc" => r.pc = v16,
            other => return Err(PyValueError::new_err(format!("unknown register {other:?}"))),
        }
        Ok(())
    }

    /// Hold the given keys for `frames` frames, then release. Keys are single
    /// characters, or the names "enter" / "space" / "caps" / "sym".
    #[pyo3(signature = (keys, frames=2))]
    fn press(&mut self, keys: Vec<String>, frames: u32) -> PyResult<()> {
        let mut held: Vec<KeyPos> = Vec::new();
        for k in &keys {
            let (pos, shift) =
                parse_key(k).ok_or_else(|| PyValueError::new_err(format!("unmapped key {k:?}")))?;
            self.spec.set_key(pos, true);
            held.push(pos);
            if let Some(s) = shift {
                self.spec.set_key(s, true);
                held.push(s);
            }
        }
        for _ in 0..frames.max(1) {
            self.spec.run_frame();
        }
        for p in held {
            self.spec.set_key(p, false);
        }
        for _ in 0..2 {
            self.spec.run_frame(); // release gap so the next press registers
        }
        Ok(())
    }

    /// Type a string through the keyboard matrix (BASIC keyword rules apply).
    fn type_text(&mut self, text: &str) -> usize {
        self.spec.type_text(text)
    }

    // --- audio (optional) ----------------------------------------------------

    /// Enable beeper capture at `sample_rate` Hz.
    fn enable_audio(&mut self, sample_rate: u32) {
        self.spec.enable_audio(sample_rate);
    }

    /// Drain captured beeper samples (mono f32, -1.0..1.0).
    fn drain_audio(&mut self) -> Vec<f32> {
        self.spec.drain_audio()
    }

    // --- session recording ---------------------------------------------------

    /// Begin capturing every `decimate`-th frame's indexed screen for video.
    #[pyo3(signature = (decimate=1))]
    fn start_recording(&mut self, decimate: u32) {
        self.spec.start_recording(decimate);
    }

    /// Stop capturing; returns the number of frames captured.
    fn stop_recording(&mut self) -> u32 {
        self.spec.stop_recording();
        self.spec.recording_count()
    }

    fn recording_count(&self) -> u32 {
        self.spec.recording_count()
    }

    fn recording_decimate(&self) -> u32 {
        self.spec.recording_decimate()
    }

    /// Take the captured frames: flattened indexed screens (256×192 bytes each,
    /// `recording_count()` of them), drained from the buffer.
    fn take_recording<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.spec.take_recording())
    }
}

// --- World of Spectrum: search + download (module-level functions) ----------

/// Search World of Spectrum for software matching `query`, best match first.
/// Returns a list of `{id, title, year, machine, publisher}` dicts.
#[pyfunction]
#[pyo3(signature = (query, limit=10))]
fn search_games<'py>(py: Python<'py>, query: &str, limit: usize) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let entries =
        py.allow_threads(|| wos::search(query, limit)).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let d = PyDict::new_bound(py);
        d.set_item("id", e.id)?;
        d.set_item("title", e.title)?;
        d.set_item("year", e.year)?;
        d.set_item("machine", e.machine)?;
        d.set_item("publisher", e.publisher)?;
        out.push(d);
    }
    Ok(out)
}

/// Find + download the best loadable build for `query`. Returns
/// `{title, year, format, data, source}` where `format` is "tap"|"z80"|"sna"
/// and `data` is the raw file bytes (load it with `Machine.autoload_tape` for
/// tap, or `Machine.load_snapshot` for snapshots). Raises if nothing loadable.
#[pyfunction]
fn fetch_game<'py>(py: Python<'py>, query: &str) -> PyResult<Bound<'py, PyDict>> {
    let g = py.allow_threads(|| wos::fetch(query)).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let d = PyDict::new_bound(py);
    d.set_item("title", g.title)?;
    d.set_item("year", g.year)?;
    d.set_item("format", g.format)?;
    d.set_item("data", PyBytes::new_bound(py, &g.data))?;
    d.set_item("source", g.source)?;
    Ok(d)
}

/// Map a key name / single char to a matrix position plus an optional shift.
fn parse_key(s: &str) -> Option<(KeyPos, Option<KeyPos>)> {
    match s.to_ascii_lowercase().as_str() {
        "enter" | "return" | "\n" => Some((keyboard::ENTER, None)),
        "space" | " " => Some((keyboard::SPACE, None)),
        "caps" | "capsshift" | "shift" => Some((keyboard::CAPS_SHIFT, None)),
        "sym" | "symshift" => Some((keyboard::SYM_SHIFT, None)),
        other => {
            let mut chars = other.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-char and not a known name
            }
            keyboard::key_for_char(ch).map(|(pos, caps, sym)| {
                let shift = if caps {
                    Some(keyboard::CAPS_SHIFT)
                } else if sym {
                    Some(keyboard::SYM_SHIFT)
                } else {
                    None
                };
                (pos, shift)
            })
        }
    }
}

#[pymodule]
fn zxspec_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Machine>()?;
    m.add_function(wrap_pyfunction!(search_games, m)?)?;
    m.add_function(wrap_pyfunction!(fetch_game, m)?)?;
    m.add("__doc__", "Headless ZX Spectrum core (Rust) exposed to Python.")?;
    Ok(())
}

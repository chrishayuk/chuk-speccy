//! The PyO3 bridge for the `ED FE` host-trap ABI: a [`PyDispatcher`] that
//! forwards each trap to a Python callable, handing it a [`TrapCtx`] with live
//! register + memory access.
//!
//! Safety: `TrapCtx` wraps raw pointers that are valid only for the duration of
//! the synchronous dispatch call. A `live` flag is flipped off when the trap
//! returns, so a retained `TrapCtx` raises loudly instead of dereferencing freed
//! state. Don't let it escape the handler (no stashing, no async).

use std::cell::Cell;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use spectrum::host::{HostCalls, HostCtx};
use spectrum::memory::Memory;
use z80::Regs;

/// Register + memory access for a Python trap handler. Created per trap; invalid
/// once the handler returns.
#[pyclass(unsendable)]
pub struct TrapCtx {
    regs: *mut Regs,
    mem: *mut Memory,
    live: Cell<bool>,
}

impl TrapCtx {
    fn guard(&self) -> PyResult<()> {
        if self.live.get() {
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("TrapCtx used after the trap returned"))
        }
    }

    #[allow(clippy::mut_from_ref)] // the raw pointer is single-threaded + live-guarded
    fn regs(&self) -> PyResult<&mut Regs> {
        self.guard()?;
        Ok(unsafe { &mut *self.regs })
    }
}

#[pymethods]
impl TrapCtx {
    /// Syscall id (register `A`).
    #[getter]
    fn a(&self) -> PyResult<u8> {
        Ok(self.regs()?.a)
    }
    #[getter]
    fn bc(&self) -> PyResult<u16> {
        Ok(self.regs()?.bc())
    }
    #[getter]
    fn de(&self) -> PyResult<u16> {
        Ok(self.regs()?.de())
    }
    #[getter]
    fn hl(&self) -> PyResult<u16> {
        Ok(self.regs()?.hl())
    }
    #[getter]
    fn ix(&self) -> PyResult<u16> {
        Ok(self.regs()?.ix)
    }
    #[getter]
    fn iy(&self) -> PyResult<u16> {
        Ok(self.regs()?.iy)
    }
    #[getter]
    fn pc(&self) -> PyResult<u16> {
        Ok(self.regs()?.pc)
    }
    #[getter]
    fn carry(&self) -> PyResult<bool> {
        Ok(self.regs()?.carry())
    }

    fn set_a(&self, v: u8) -> PyResult<()> {
        self.regs()?.a = v;
        Ok(())
    }
    fn set_bc(&self, v: u16) -> PyResult<()> {
        self.regs()?.set_bc(v);
        Ok(())
    }
    fn set_de(&self, v: u16) -> PyResult<()> {
        self.regs()?.set_de(v);
        Ok(())
    }
    fn set_hl(&self, v: u16) -> PyResult<()> {
        self.regs()?.set_hl(v);
        Ok(())
    }
    fn set_ix(&self, v: u16) -> PyResult<()> {
        self.regs()?.ix = v;
        Ok(())
    }
    fn set_iy(&self, v: u16) -> PyResult<()> {
        self.regs()?.iy = v;
        Ok(())
    }
    /// Carry = error convention (True ⇒ the trap failed).
    fn set_carry(&self, err: bool) -> PyResult<()> {
        self.regs()?.set_carry(err);
        Ok(())
    }

    /// Read `len` bytes from `addr`.
    fn read<'py>(&self, py: Python<'py>, addr: u16, len: u16) -> PyResult<Bound<'py, PyBytes>> {
        self.guard()?;
        let mem = unsafe { &*self.mem };
        let bytes: Vec<u8> = (0..len).map(|i| mem.read(addr.wrapping_add(i))).collect();
        Ok(PyBytes::new_bound(py, &bytes))
    }

    /// Write `data` into memory at `addr` (ROM writes ignored).
    fn write(&self, addr: u16, data: Vec<u8>) -> PyResult<()> {
        self.guard()?;
        let mem = unsafe { &mut *self.mem };
        for (i, &b) in data.iter().enumerate() {
            mem.write(addr.wrapping_add(i as u16), b);
        }
        Ok(())
    }
}

/// Forwards every host trap to a Python callable `cb(ctx)`.
pub struct PyDispatcher {
    cb: Py<PyAny>,
}

impl PyDispatcher {
    pub fn new(cb: Py<PyAny>) -> Self {
        Self { cb }
    }
}

impl HostCalls for PyDispatcher {
    fn dispatch(&mut self, ctx: &mut HostCtx) -> u32 {
        let (regs, mem) = ctx.raw_parts();
        Python::with_gil(|py| {
            let obj = match Py::new(py, TrapCtx { regs, mem, live: Cell::new(true) }) {
                Ok(o) => o,
                Err(_) => return,
            };
            if let Err(e) = self.cb.call1(py, (obj.clone_ref(py),)) {
                e.print(py); // a failing handler shouldn't poison the emulator
            }
            obj.borrow(py).live.set(false); // invalidate the ctx after the call
        });
        0
    }
}

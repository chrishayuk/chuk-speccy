//! The per-function "register file": named locals (and parameters) mapped to scratch
//! slots. Flat scoping; arrays occupy one 2-byte slot per element. A variable also
//! remembers its value/element [`Width`], whether it is a struct (and which), whether
//! it is a pointer receiver (`self`), and whether it is a prelude handle (`Frame`).

use crate::ir::Width;
use std::collections::HashMap;

struct VarInfo {
    base: usize,
    sty: Option<String>,
    ty: Width,    // scalar value type, or array element type
    is_ptr: bool, // a pointer to a struct (e.g. `self`) vs a by-value struct local
    /// A prelude handle type (`"Frame"`/`"Input"`) — methods route to intrinsics.
    handle: Option<String>,
    /// For a struct-element array (`[Cell; N]`), the element struct's name — so element
    /// access (`a[i].x`) knows the element stride + field layout.
    elem_struct: Option<String>,
}

/// Name → variable info. `next` is the next free slot.
#[derive(Default)]
pub(crate) struct Vars {
    map: HashMap<String, VarInfo>,
    pub(crate) next: usize,
}

impl Vars {
    pub(crate) fn declare(
        &mut self,
        name: &str,
        size: usize,
        sty: Option<String>,
        ty: Width,
    ) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty,
                ty,
                is_ptr: false,
                handle: None,
                elem_struct: None,
            },
        );
        self.next += size;
        base
    }
    /// Declare a pointer-to-struct local (one slot holding an address) — `self`.
    pub(crate) fn declare_ptr(&mut self, name: &str, sty: &str) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty: Some(sty.to_string()),
                ty: Width::Word,
                is_ptr: true,
                handle: None,
                elem_struct: None,
            },
        );
        self.next += 1;
        base
    }
    /// Declare a prelude-handle param (`frame: &mut Frame`, `input: &Input`).
    pub(crate) fn declare_handle(&mut self, name: &str, handle: &str) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty: None,
                ty: Width::Word,
                is_ptr: false,
                handle: Some(handle.to_string()),
                elem_struct: None,
            },
        );
        self.next += 1;
        base
    }
    pub(crate) fn handle_of(&self, name: &str) -> Option<String> {
        self.map.get(name).and_then(|v| v.handle.clone())
    }
    pub(crate) fn base(&mut self, name: &str) -> usize {
        match self.map.get(name) {
            Some(v) => v.base,
            None => self.declare(name, 1, None, Width::Word),
        }
    }
    /// A struct-typed var as a method receiver: `(base, struct name, is_ptr)`.
    pub(crate) fn receiver(&self, name: &str) -> Option<(usize, String, bool)> {
        self.map
            .get(name)
            .and_then(|v| v.sty.as_ref().map(|s| (v.base, s.clone(), v.is_ptr)))
    }
    /// The variable's value type (scalar) or element type (array).
    pub(crate) fn ty(&self, name: &str) -> Width {
        self.map.get(name).map(|v| v.ty).unwrap_or(Width::Word)
    }
    /// Declare a struct-element array (`[Cell; N]`): `slots` total slots, remembering
    /// the element struct so `a[i].field` can compute the element address.
    pub(crate) fn declare_struct_array(
        &mut self,
        name: &str,
        slots: usize,
        elem_struct: &str,
    ) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty: None,
                ty: Width::Word,
                is_ptr: false,
                handle: None,
                elem_struct: Some(elem_struct.to_string()),
            },
        );
        self.next += slots;
        base
    }
    /// The element struct of a struct-element array var, if it is one.
    pub(crate) fn elem_struct(&self, name: &str) -> Option<String> {
        self.map.get(name).and_then(|v| v.elem_struct.clone())
    }
}

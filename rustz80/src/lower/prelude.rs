//! Handle routing — the one hook that keeps the compiler generic. A "handle"
//! parameter (e.g. an SDK `Frame`/`Input`) routes its method calls to free prelude
//! functions, *dropping* the receiver (the 3-register calling convention has no room
//! for `self` + 3 args). The compiler knows nothing about any particular SDK; the
//! caller supplies the `(handle type, method) → fn` map. Generic compilation passes
//! an empty config — then there are no handle types.

use std::collections::HashMap;

#[derive(Default, Clone)]
pub struct PreludeConfig {
    /// `(handle type name, method) → prelude function name`.
    routes: HashMap<(String, String), String>,
}

impl PreludeConfig {
    pub fn new() -> Self {
        Self::default()
    }
    /// Route `<handle>.<method>(args)` to `fn_name(args)` (receiver dropped).
    pub fn route(mut self, handle: &str, method: &str, fn_name: &str) -> Self {
        self.routes.insert(
            (handle.to_string(), method.to_string()),
            fn_name.to_string(),
        );
        self
    }
    /// Is `ty` a handle type (any method of it routes to a prelude fn)?
    fn is_handle(&self, ty: &str) -> bool {
        self.routes.keys().any(|(h, _)| h == ty)
    }
    /// The prelude fn for `<handle>.<method>`, if routed.
    pub(crate) fn lookup(&self, handle: &str, method: &str) -> Option<&str> {
        self.routes
            .get(&(handle.to_string(), method.to_string()))
            .map(String::as_str)
    }
}

/// A handle parameter type (`&mut T`/`&T`/`T` → `"T"`) if `T` is a configured handle
/// type (e.g. the SDK's `Frame`/`Input`); otherwise `None`.
pub(crate) fn handle_type(t: &syn::Type, prelude: &PreludeConfig) -> Option<String> {
    let inner = match t {
        syn::Type::Reference(r) => &*r.elem,
        other => other,
    };
    if let syn::Type::Path(p) = inner {
        if let Some(id) = p.path.get_ident() {
            let s = id.to_string();
            if prelude.is_handle(&s) {
                return Some(s);
            }
        }
    }
    None
}

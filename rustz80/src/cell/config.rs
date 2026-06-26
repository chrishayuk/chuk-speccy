//! Cell safety policy + capability scan.

/// Safety policy for a cell. Games need raw memory; general agent cells usually do not —
/// so the intrinsics are **capability-gated, off by default** ([`CellConfig::sandboxed`]),
/// and resource ceilings are explicit. The cycle budget (passed to [`Runner::run`]) is the
/// deterministic liveness guard; these are the rest.
#[derive(Debug, Clone)]
pub struct CellConfig {
    /// Allow `poke`/`peek` (raw memory access).
    pub allow_raw_memory: bool,
    /// Allow `inport` (I/O ports).
    pub allow_ports: bool,
    /// Reject if the compiled code exceeds this many bytes.
    pub max_code_bytes: Option<usize>,
    /// Abort the run if it writes more than this many distinct addresses.
    pub max_touched: Option<usize>,
}

impl CellConfig {
    /// Deny raw memory + ports, with tight ceilings — the default for untrusted cells.
    pub fn sandboxed() -> Self {
        CellConfig {
            allow_raw_memory: false,
            allow_ports: false,
            max_code_bytes: Some(4096),
            max_touched: Some(4096),
        }
    }
    /// Allow everything, no ceilings — for trusted/game code (matches the pre-policy
    /// behaviour).
    pub fn permissive() -> Self {
        CellConfig {
            allow_raw_memory: true,
            allow_ports: true,
            max_code_bytes: None,
            max_touched: None,
        }
    }
}

impl Default for CellConfig {
    /// Safe by default.
    fn default() -> Self {
        Self::sandboxed()
    }
}

/// Which capability-gated intrinsics a source uses (`poke`/`peek`/`inport`).
#[derive(Default)]
struct Caps {
    raw_memory: bool, // poke / peek
    ports: bool,      // inport
}

impl<'ast> syn::visit::Visit<'ast> for Caps {
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            match p.path.get_ident().map(|i| i.to_string()).as_deref() {
                Some("poke") | Some("peek") => self.raw_memory = true,
                Some("inport") => self.ports = true,
                _ => {}
            }
        }
        syn::visit::visit_expr_call(self, c); // recurse into nested calls
    }
}

/// Check a parsed file against a config's capability gates (walks for the gated
/// intrinsics).
pub(super) fn check_caps(file: &syn::File, cfg: &CellConfig) -> Result<(), String> {
    let mut caps = Caps::default();
    syn::visit::visit_file(&mut caps, file);
    if caps.raw_memory && !cfg.allow_raw_memory {
        return Err("raw memory (`poke`/`peek`) is not allowed (enable allow_raw_memory)".into());
    }
    if caps.ports && !cfg.allow_ports {
        return Err("I/O ports (`inport`) are not allowed (enable allow_ports)".into());
    }
    Ok(())
}

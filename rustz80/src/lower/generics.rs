//! Generics by monomorphization. A generic free fn (`fn max<T>(a: T, b: T) -> T`) is
//! *not* lowered eagerly. Each call instantiates a width-specialized copy: the type
//! parameter resolves to a concrete [`Width`] (from a turbofish or inferred from the
//! argument widths), the instance's params are declared at that width, and the body
//! lowers exactly as a normal function. Codegen is untouched — instances are just
//! extra named functions (`max$u16`, `max$u8`).

use super::Ctx;
use crate::ir::{Expr, Width};
use std::collections::{HashMap, HashSet};

/// A generic free function awaiting instantiation.
#[derive(Clone)]
pub(crate) struct GenericFn {
    pub(crate) item: syn::ItemFn,
    /// Type-parameter names, in declaration order.
    pub(crate) params: Vec<String>,
    /// For each type-parameter name, the index of the first value parameter whose
    /// type *is* that parameter — used to infer a type argument from the call args.
    source: HashMap<String, usize>,
    /// The return type, when it is itself a type parameter (so the call's result
    /// width follows the instantiation).
    ret_param: Option<String>,
}

/// One requested monomorphic instance.
pub(crate) struct Instance {
    pub(crate) generic: String,
    pub(crate) type_args: Vec<Width>,
    pub(crate) name: String,
}

/// Shared monomorphization state: the generic registry, a worklist of instances to
/// lower, and the set already requested (so each instance is emitted once).
#[derive(Default)]
pub(crate) struct Mono {
    pub(crate) generics: HashMap<String, GenericFn>,
    pub(crate) queue: Vec<Instance>,
    seen: HashSet<String>,
}

impl Mono {
    /// A registry seeded with the program's generic functions, empty worklist.
    pub(crate) fn new(generics: HashMap<String, GenericFn>) -> Self {
        Mono {
            generics,
            queue: Vec::new(),
            seen: HashSet::new(),
        }
    }
    /// Request an instance (idempotent); returns its mangled symbol name.
    pub(crate) fn request(&mut self, generic: &str, type_args: Vec<Width>) -> String {
        let name = instance_name(generic, &type_args);
        if self.seen.insert(name.clone()) {
            self.queue.push(Instance {
                generic: generic.to_string(),
                type_args,
                name: name.clone(),
            });
        }
        name
    }
}

fn width_tag(w: Width) -> &'static str {
    match w {
        Width::Byte => "u8",
        Width::Word => "u16",
    }
}

/// A unique symbol name for one instantiation, e.g. `max$u16` / `add3$u8`.
fn instance_name(generic: &str, args: &[Width]) -> String {
    let tags: Vec<&str> = args.iter().map(|w| width_tag(*w)).collect();
    format!("{generic}${}", tags.join("_"))
}

/// Does this signature declare any type parameter (i.e. is it generic)?
pub(crate) fn is_generic_sig(sig: &syn::Signature) -> bool {
    sig.generics.type_params().next().is_some()
}

pub(crate) fn is_generic_fn(f: &syn::ItemFn) -> bool {
    is_generic_sig(&f.sig)
}

/// If `t` is exactly one of the named type parameters, return its name.
fn type_param_of(t: &syn::Type, params: &[String]) -> Option<String> {
    if let syn::Type::Path(p) = t {
        if let Some(id) = p.path.get_ident() {
            let s = id.to_string();
            if params.contains(&s) {
                return Some(s);
            }
        }
    }
    None
}

/// Collect generic free functions for on-demand monomorphization. Only type
/// parameters are supported (lifetimes/const generics are rejected); bounds and
/// `where` clauses are ignored (rustc already checked them).
pub(crate) fn collect_generic_fns(file: &syn::File) -> Result<HashMap<String, GenericFn>, String> {
    let mut map = HashMap::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        if !is_generic_fn(f) {
            continue;
        }
        if f.sig.generics.lifetimes().next().is_some()
            || f.sig.generics.const_params().next().is_some()
        {
            return Err(format!(
                "`{}`: only type parameters are supported on generic functions",
                f.sig.ident
            ));
        }
        let params: Vec<String> = f
            .sig
            .generics
            .type_params()
            .map(|tp| tp.ident.to_string())
            .collect();

        // Map each type parameter to the first value parameter declared with it.
        let mut source = HashMap::new();
        for (i, arg) in f.sig.inputs.iter().enumerate() {
            if let syn::FnArg::Typed(pt) = arg {
                if let Some(p) = type_param_of(&pt.ty, &params) {
                    source.entry(p).or_insert(i);
                }
            }
        }
        let ret_param = match &f.sig.output {
            syn::ReturnType::Type(_, t) => type_param_of(t, &params),
            syn::ReturnType::Default => None,
        };
        map.insert(
            f.sig.ident.to_string(),
            GenericFn {
                item: f.clone(),
                params,
                source,
                ret_param,
            },
        );
    }
    Ok(map)
}

/// Extract a call's target name and any turbofish type arguments (`f::<u8>(…)`),
/// tolerating a path with generic arguments (which `path_ident` rejects).
pub(crate) fn call_target(func: &syn::Expr) -> Result<(String, Vec<syn::Type>), String> {
    let syn::Expr::Path(p) = func else {
        return Err(format!("unsupported call target: {func:?}"));
    };
    let seg = p.path.segments.last().ok_or("empty call path")?;
    let name = seg.ident.to_string();
    let types = match &seg.arguments {
        syn::PathArguments::None => Vec::new(),
        syn::PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .map(|a| match a {
                syn::GenericArgument::Type(t) => Ok(t.clone()),
                _ => Err("only type arguments are supported in `::<…>`".to_string()),
            })
            .collect::<Result<_, _>>()?,
        syn::PathArguments::Parenthesized(_) => {
            return Err("Fn-trait call syntax is not supported".into())
        }
    };
    Ok((name, types))
}

/// Resolve a generic call to its concrete type-argument widths and result width.
/// Widths come from the turbofish if present, else are inferred from the argument
/// whose parameter has that type. `lowered` is the call's already-lowered args.
pub(crate) fn resolve_generic(
    name: &str,
    turbofish: &[syn::Type],
    lowered: &[(Expr, Width)],
    ctx: &Ctx,
) -> Result<(Vec<Width>, Width), String> {
    let m = ctx.mono.borrow();
    let gf = &m.generics[name];

    let type_args: Vec<Width> = if turbofish.is_empty() {
        // Infer each type parameter from the first argument declared with that type.
        gf.params
            .iter()
            .map(|p| {
                let idx = gf.source.get(p).ok_or_else(|| {
                    format!(
                        "cannot infer type argument `{p}` of `{name}` — add a turbofish `::<…>`"
                    )
                })?;
                let (_, w) = lowered
                    .get(*idx)
                    .ok_or_else(|| format!("too few arguments to `{name}`"))?;
                Ok(*w)
            })
            .collect::<Result<_, String>>()?
    } else {
        if turbofish.len() != gf.params.len() {
            return Err(format!(
                "`{name}` takes {} type argument(s), got {}",
                gf.params.len(),
                turbofish.len()
            ));
        }
        turbofish.iter().map(|t| ctx.width_of_type(t)).collect()
    };

    // The result width follows a generic return type, else the concrete annotation.
    let ret_w = match &gf.ret_param {
        Some(p) => {
            let pos = gf.params.iter().position(|x| x == p).unwrap();
            type_args[pos]
        }
        None => match &gf.item.sig.output {
            syn::ReturnType::Type(_, t) => ctx.width_of_type(t),
            syn::ReturnType::Default => Width::Word,
        },
    };
    Ok((type_args, ret_w))
}

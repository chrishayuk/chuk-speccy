//! Generics by monomorphization. A generic free fn (`fn max<T>(…)` / `fn buf<const N:
//! usize>()`) is *not* lowered eagerly. Each call instantiates a specialized copy: a
//! type parameter resolves to a concrete [`Width`] (turbofish or inferred from the
//! argument widths), a const parameter to a concrete value (turbofish only). The
//! instance's params are declared at those widths and const values substituted in, so
//! the body lowers exactly as a normal function. Codegen is untouched — instances are
//! just extra named functions (`max$u16`, `buf$8`).

use super::Ctx;
use crate::ir::{Expr, Width};
use std::collections::{HashMap, HashSet};

/// A generic parameter's kind.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamKind {
    Type,
    Const,
}

/// One generic parameter (in declaration order).
#[derive(Clone)]
pub(crate) struct Param {
    pub(crate) name: String,
    pub(crate) kind: ParamKind,
}

/// A concrete generic argument: a type's width, or a const value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GArg {
    Width(Width),
    Const(u16),
}

/// A generic free function awaiting instantiation.
#[derive(Clone)]
pub(crate) struct GenericFn {
    pub(crate) item: syn::ItemFn,
    /// Generic parameters, in declaration order.
    pub(crate) params: Vec<Param>,
    /// For each *type* parameter, the index of the first value parameter whose type
    /// *is* that parameter — used to infer a type argument from the call's args.
    source: HashMap<String, usize>,
    /// The return type, when it is itself a type parameter (so the call's result width
    /// follows the instantiation).
    ret_param: Option<String>,
}

/// One requested monomorphic instance.
pub(crate) struct Instance {
    pub(crate) generic: String,
    pub(crate) args: Vec<GArg>,
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
    pub(crate) fn request(&mut self, generic: &str, args: Vec<GArg>) -> String {
        let name = instance_name(generic, &args);
        if self.seen.insert(name.clone()) {
            self.queue.push(Instance {
                generic: generic.to_string(),
                args,
                name: name.clone(),
            });
        }
        name
    }
}

fn arg_tag(a: GArg) -> String {
    match a {
        GArg::Width(Width::Byte) => "u8".to_string(),
        GArg::Width(Width::Word) => "u16".to_string(),
        GArg::Const(n) => n.to_string(),
    }
}

/// A unique symbol name for one instantiation, e.g. `max$u16` / `buf$8` / `zip$u16_4`.
fn instance_name(generic: &str, args: &[GArg]) -> String {
    let tags: Vec<String> = args.iter().map(|a| arg_tag(*a)).collect();
    format!("{generic}${}", tags.join("_"))
}

/// Does this signature declare any type or const parameter (i.e. is it generic)?
pub(crate) fn is_generic_sig(sig: &syn::Signature) -> bool {
    sig.generics.type_params().next().is_some() || sig.generics.const_params().next().is_some()
}

pub(crate) fn is_generic_fn(f: &syn::ItemFn) -> bool {
    is_generic_sig(&f.sig)
}

/// If `t` is exactly one of the named type parameters, return its name.
fn type_param_of(t: &syn::Type, type_names: &[String]) -> Option<String> {
    if let syn::Type::Path(p) = t {
        if let Some(id) = p.path.get_ident() {
            let s = id.to_string();
            if type_names.contains(&s) {
                return Some(s);
            }
        }
    }
    None
}

/// Collect generic free functions for on-demand monomorphization. Type and const
/// parameters are supported (lifetimes are rejected); bounds and `where` clauses are
/// ignored (rustc already checked them).
pub(crate) fn collect_generic_fns(file: &syn::File) -> Result<HashMap<String, GenericFn>, String> {
    let mut map = HashMap::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        if !is_generic_fn(f) {
            continue;
        }
        if f.sig.generics.lifetimes().next().is_some() {
            return Err(format!(
                "`{}`: lifetime parameters are not supported",
                f.sig.ident
            ));
        }
        let params: Vec<Param> = f
            .sig
            .generics
            .params
            .iter()
            .filter_map(|p| match p {
                syn::GenericParam::Type(tp) => Some(Param {
                    name: tp.ident.to_string(),
                    kind: ParamKind::Type,
                }),
                syn::GenericParam::Const(cp) => Some(Param {
                    name: cp.ident.to_string(),
                    kind: ParamKind::Const,
                }),
                syn::GenericParam::Lifetime(_) => None,
            })
            .collect();
        let type_names: Vec<String> = params
            .iter()
            .filter(|p| p.kind == ParamKind::Type)
            .map(|p| p.name.clone())
            .collect();

        // Map each type parameter to the first value parameter declared with it.
        let mut source = HashMap::new();
        for (i, arg) in f.sig.inputs.iter().enumerate() {
            if let syn::FnArg::Typed(pt) = arg {
                if let Some(p) = type_param_of(&pt.ty, &type_names) {
                    source.entry(p).or_insert(i);
                }
            }
        }
        let ret_param = match &f.sig.output {
            syn::ReturnType::Type(_, t) => type_param_of(t, &type_names),
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

/// Extract a call's target name and any turbofish arguments (`f::<u8, 4>(…)`),
/// tolerating a path with generic arguments (which `path_ident` rejects).
pub(crate) fn call_target(func: &syn::Expr) -> Result<(String, Vec<syn::GenericArgument>), String> {
    let syn::Expr::Path(p) = func else {
        return Err(format!("unsupported call target: {func:?}"));
    };
    let seg = p.path.segments.last().ok_or("empty call path")?;
    let name = seg.ident.to_string();
    let args = match &seg.arguments {
        syn::PathArguments::None => Vec::new(),
        syn::PathArguments::AngleBracketed(ab) => ab.args.iter().cloned().collect(),
        syn::PathArguments::Parenthesized(_) => {
            return Err("Fn-trait call syntax is not supported".into())
        }
    };
    Ok((name, args))
}

/// Evaluate a const-generic argument to a `u16` (it must be an integer literal).
fn eval_const(e: &syn::Expr) -> Result<u16, String> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<u16>().map_err(|e| e.to_string());
        }
    }
    Err("a const generic argument must be an integer literal".into())
}

/// Resolve a generic call to its concrete arguments and result width. Type widths come
/// from the turbofish or are inferred from the argument whose parameter has that type;
/// const values come from the turbofish (they cannot be inferred). `lowered` is the
/// call's already-lowered value arguments.
pub(crate) fn resolve_generic(
    name: &str,
    turbofish: &[syn::GenericArgument],
    lowered: &[(Expr, Width)],
    ctx: &Ctx,
) -> Result<(Vec<GArg>, Width), String> {
    let m = ctx.mono.borrow();
    let gf = &m.generics[name];

    let args: Vec<GArg> = if turbofish.is_empty() {
        gf.params
            .iter()
            .map(|p| match p.kind {
                ParamKind::Type => {
                    let idx = gf.source.get(&p.name).ok_or_else(|| {
                        format!(
                            "cannot infer type argument `{}` of `{name}` — add a turbofish `::<…>`",
                            p.name
                        )
                    })?;
                    let (_, w) = lowered
                        .get(*idx)
                        .ok_or_else(|| format!("too few arguments to `{name}`"))?;
                    Ok(GArg::Width(*w))
                }
                ParamKind::Const => Err(format!(
                    "const argument `{}` of `{name}` needs a turbofish `::<…>`",
                    p.name
                )),
            })
            .collect::<Result<_, String>>()?
    } else {
        if turbofish.len() != gf.params.len() {
            return Err(format!(
                "`{name}` takes {} generic argument(s), got {}",
                gf.params.len(),
                turbofish.len()
            ));
        }
        gf.params
            .iter()
            .zip(turbofish)
            .map(|(p, ga)| match (p.kind, ga) {
                (ParamKind::Type, syn::GenericArgument::Type(t)) => {
                    Ok(GArg::Width(ctx.width_of_type(t)))
                }
                (ParamKind::Const, syn::GenericArgument::Const(e)) => {
                    Ok(GArg::Const(eval_const(e)?))
                }
                _ => Err(format!("generic argument kind mismatch for `{name}`")),
            })
            .collect::<Result<_, String>>()?
    };

    // The result width follows a generic return type, else the concrete annotation.
    let ret_w = match &gf.ret_param {
        Some(p) => {
            let pos = gf.params.iter().position(|x| &x.name == p).unwrap();
            match args[pos] {
                GArg::Width(w) => w,
                GArg::Const(_) => Width::Word,
            }
        }
        None => match &gf.item.sig.output {
            syn::ReturnType::Type(_, t) => ctx.width_of_type(t),
            syn::ReturnType::Default => Width::Word,
        },
    };
    Ok((args, ret_w))
}

//! Generics by monomorphization. A generic free fn (`fn max<T>(…)` / `fn buf<const N:
//! usize>()`) is *not* lowered eagerly. Each call instantiates a specialized copy: a
//! type parameter resolves to a concrete [`Width`] (turbofish or inferred from the
//! argument widths), a const parameter to a concrete value (turbofish only). The
//! instance's params are declared at those widths and const values substituted in, so
//! the body lowers exactly as a normal function. Codegen is untouched — instances are
//! just extra named functions (`max$u16`, `buf$8`).

use super::layout::{struct_field_defs, FieldDef};
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

/// A generic free function (or generic-struct method) awaiting instantiation.
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
    /// The base struct name when this is a generic-struct method (`impl<…> Buf<…> { fn
    /// m(&self) }` → `Some("Buf")`); the instance lowers with `self` typed as the
    /// matching struct instance (`Buf$8`).
    pub(crate) self_ty: Option<String>,
}

/// A const-generic (or generic) struct definition awaiting instantiation.
#[derive(Clone)]
pub(crate) struct GenericStruct {
    pub(crate) item: syn::ItemStruct,
    pub(crate) params: Vec<Param>,
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
    /// Const-/generic struct definitions, by base name.
    pub(crate) generic_structs: HashMap<String, GenericStruct>,
    /// Concrete per-instance struct layouts, by mangled name (`Buf$8`), registered on
    /// demand at construction.
    pub(crate) struct_instances: HashMap<String, Vec<FieldDef>>,
    pub(crate) queue: Vec<Instance>,
    seen: HashSet<String>,
}

impl Mono {
    /// A registry seeded with the program's generic functions, empty worklist.
    pub(crate) fn new(generics: HashMap<String, GenericFn>) -> Self {
        Mono {
            generics,
            ..Mono::default()
        }
    }
    /// Request an instance (idempotent); returns its mangled symbol name.
    pub(crate) fn request(&mut self, generic: &str, args: Vec<GArg>) -> String {
        let name = instance_name(generic, &args);
        self.request_named(generic, args, name.clone());
        name
    }
    /// Request an instance under an explicit symbol name (for struct methods, named
    /// `Buf$8::push` so they match the call site).
    fn request_named(&mut self, generic: &str, args: Vec<GArg>, name: String) {
        if self.seen.insert(name.clone()) {
            self.queue.push(Instance {
                generic: generic.to_string(),
                args,
                name,
            });
        }
    }

    /// Instantiate a const-/generic struct at concrete `args`: register its per-instance
    /// layout (resolving `[u16; N]` sizes) and request each of its methods. Returns the
    /// mangled struct name (`Buf$8`).
    pub(crate) fn instantiate_struct(
        &mut self,
        name: &str,
        args: Vec<GArg>,
    ) -> Result<String, String> {
        let mangled = instance_name(name, &args);
        if self.struct_instances.contains_key(&mangled) {
            return Ok(mangled);
        }
        let gs = self
            .generic_structs
            .get(name)
            .ok_or_else(|| format!("unknown generic struct {name}"))?
            .clone();
        let consts = const_map(&gs.params, &args);
        let syn::Fields::Named(named) = &gs.item.fields else {
            return Err(format!("only named-field structs are supported: {name}"));
        };
        // Element-struct lookup for a const-generic struct's `[Cell; N]` field would
        // need the regular layout map (the generic combo is a later step); pass empty.
        let layout = struct_field_defs(named, &consts, &Default::default(), name)?;
        self.struct_instances.insert(mangled.clone(), layout);

        // Request each method as an instance named `Buf$8::method`.
        let methods: Vec<(String, String)> = self
            .generics
            .iter()
            .filter(|(_, gf)| gf.self_ty.as_deref() == Some(name))
            .map(|(key, _)| (key.clone(), key.rsplit("::").next().unwrap().to_string()))
            .collect();
        for (key, m) in methods {
            self.request_named(&key, args.clone(), format!("{mangled}::{m}"));
        }
        Ok(mangled)
    }
}

/// Map each const parameter to its concrete value, for resolving `[u16; N]` lengths.
fn const_map(params: &[Param], args: &[GArg]) -> HashMap<String, u16> {
    params
        .iter()
        .zip(args)
        .filter_map(|(p, a)| match a {
            GArg::Const(n) => Some((p.name.clone(), *n)),
            GArg::Width(_) => None,
        })
        .collect()
}

fn arg_tag(a: GArg) -> String {
    match a {
        GArg::Width(Width::Byte) => "u8".to_string(),
        GArg::Width(Width::Word) => "u16".to_string(),
        GArg::Const(n) => n.to_string(),
    }
}

/// A unique symbol name for one instantiation, e.g. `max$u16` / `buf$8` / `zip$u16_4`.
pub(crate) fn instance_name(generic: &str, args: &[GArg]) -> String {
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

/// The type/const parameters of a `syn::Generics` (lifetimes are rejected).
fn params_of(generics: &syn::Generics, owner: &str) -> Result<Vec<Param>, String> {
    if generics.lifetimes().next().is_some() {
        return Err(format!("`{owner}`: lifetime parameters are not supported"));
    }
    Ok(generics
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
        .collect())
}

/// Build a [`GenericFn`] from a signature + body (a free fn or a generic-struct
/// method). `self_ty` is the base struct name for a method.
fn generic_fn(item: syn::ItemFn, params: Vec<Param>, self_ty: Option<String>) -> GenericFn {
    let type_names: Vec<String> = params
        .iter()
        .filter(|p| p.kind == ParamKind::Type)
        .map(|p| p.name.clone())
        .collect();
    // Map each type parameter to the first value parameter declared with it.
    let mut source = HashMap::new();
    for (i, arg) in item.sig.inputs.iter().enumerate() {
        if let syn::FnArg::Typed(pt) = arg {
            if let Some(p) = type_param_of(&pt.ty, &type_names) {
                source.entry(p).or_insert(i);
            }
        }
    }
    let ret_param = match &item.sig.output {
        syn::ReturnType::Type(_, t) => type_param_of(t, &type_names),
        syn::ReturnType::Default => None,
    };
    GenericFn {
        item,
        params,
        source,
        ret_param,
        self_ty,
    }
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
        let name = f.sig.ident.to_string();
        let params = params_of(&f.sig.generics, &name)?;
        map.insert(name, generic_fn(f.clone(), params, None));
    }
    Ok(map)
}

/// Collect const-generic struct definitions (those with a const parameter — their
/// layout is per-instance). Type-param-only structs stay in the regular layout map.
pub(crate) fn collect_generic_structs(
    file: &syn::File,
) -> Result<HashMap<String, GenericStruct>, String> {
    let mut map = HashMap::new();
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            if s.generics.const_params().next().is_some() {
                let name = s.ident.to_string();
                let params = params_of(&s.generics, &name)?;
                map.insert(
                    name,
                    GenericStruct {
                        item: s.clone(),
                        params,
                    },
                );
            }
        }
    }
    Ok(map)
}

/// Is the impl block for one of the given const-generic structs?
pub(crate) fn impl_is_for_generic_struct(
    imp: &syn::ItemImpl,
    structs: &HashMap<String, GenericStruct>,
) -> bool {
    super::layout::type_name(&imp.self_ty)
        .map(|n| structs.contains_key(&n))
        .unwrap_or(false)
}

/// Collect the methods of const-generic structs into the generic-fn map, keyed
/// `Struct::method` with the impl's parameters and `self_ty = Some(Struct)`.
pub(crate) fn collect_generic_methods(
    file: &syn::File,
    structs: &HashMap<String, GenericStruct>,
    out: &mut HashMap<String, GenericFn>,
) -> Result<(), String> {
    for item in &file.items {
        let syn::Item::Impl(imp) = item else { continue };
        if !impl_is_for_generic_struct(imp, structs) {
            continue;
        }
        let base = super::layout::type_name(&imp.self_ty)?;
        let params = params_of(&imp.generics, &base)?;
        for it in &imp.items {
            let syn::ImplItem::Fn(m) = it else {
                return Err("only methods are supported in impl blocks".into());
            };
            let item = syn::ItemFn {
                attrs: m.attrs.clone(),
                vis: m.vis.clone(),
                sig: m.sig.clone(),
                block: Box::new(m.block.clone()),
            };
            let key = format!("{base}::{}", m.sig.ident);
            out.insert(key, generic_fn(item, params.clone(), Some(base.clone())));
        }
    }
    Ok(())
}

/// Infer a generic struct's arguments from a struct literal: a const parameter is read
/// from the `[v; LEN]` array field whose declared length is that parameter; a type
/// parameter is erased to 16-bit.
pub(crate) fn infer_struct_args(
    gs: &GenericStruct,
    lit: &syn::ExprStruct,
    ctx: &Ctx,
) -> Result<Vec<GArg>, String> {
    let syn::Fields::Named(named) = &gs.item.fields else {
        return Err("only named-field structs are supported".into());
    };
    gs.params
        .iter()
        .map(|p| match p.kind {
            ParamKind::Type => Ok(GArg::Width(Width::Word)),
            ParamKind::Const => {
                // Find the array field `[_; p.name]` and read its length from the literal.
                let fname = named
                    .named
                    .iter()
                    .find_map(|f| match &f.ty {
                        syn::Type::Array(arr) if path_is(&arr.len, &p.name) => {
                            f.ident.as_ref().map(|i| i.to_string())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        format!("cannot infer const `{}` of `{}`", p.name, gs.item.ident)
                    })?;
                let fv = lit
                    .fields
                    .iter()
                    .find(|fv| matches!(&fv.member, syn::Member::Named(n) if *n == fname))
                    .ok_or_else(|| format!("field `{fname}` missing in literal"))?;
                let syn::Expr::Repeat(r) = &fv.expr else {
                    return Err(format!("field `{fname}` must be initialised `[v; N]`"));
                };
                Ok(GArg::Const(ctx.eval_len(&r.len)?))
            }
        })
        .collect()
}

/// Is `e` a path equal to `name` (a const-param length expression)?
fn path_is(e: &syn::Expr, name: &str) -> bool {
    matches!(e, syn::Expr::Path(p) if p.path.is_ident(name))
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

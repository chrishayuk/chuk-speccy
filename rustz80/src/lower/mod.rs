//! Lower a `syn` file (the accepted subset) to the IR. Unsupported nodes become
//! errors — the "not supported on Z80 / host-only" signal.
//!
//! This module owns the lowering [`Ctx`] and the function-level orchestration
//! (`lower_program`, generic instantiation, parameters, the function body). The work
//! is split across submodules by concern:
//!
//! - [`vars`] — the per-function register file (named locals → slots);
//! - [`layout`] — struct/enum layout + the syntactic parse helpers;
//! - [`prelude`] — handle routing ([`PreludeConfig`]), the generic-compiler hook;
//! - [`generics`] — monomorphization of generic free functions;
//! - [`expr`] — expression lowering (and field/index/method access);
//! - [`stmt`] — statements and control flow (`if`/`while`/`for`/`loop`/`match`).

mod expr;
mod generics;
mod layout;
mod prelude;
mod stmt;
mod vars;

pub use prelude::PreludeConfig;

use crate::ir::*;
use expr::lower_expr;
use generics::{
    collect_generic_fns, collect_generic_methods, collect_generic_structs,
    impl_is_for_generic_struct, instance_name, is_generic_fn, is_generic_sig, GArg, Mono,
};
use layout::{collect_enums, collect_structs, type_name, Enums, FieldDef, Structs};
use prelude::handle_type;
use std::cell::RefCell;
use std::collections::HashMap;
use stmt::{lower_local, lower_stmt_expr, pat_ident};
use vars::Vars;

/// Per-function lowering context: locals + the program's struct/enum layouts + the
/// caller's handle-routing config + the (shared) monomorphization state.
pub(crate) struct Ctx<'a> {
    pub(crate) vars: Vars,
    pub(crate) structs: &'a Structs,
    pub(crate) enums: &'a Enums,
    pub(crate) prelude: &'a PreludeConfig,
    /// Counter for synthesised `match`/`for` temporaries.
    pub(crate) temp: usize,
    /// Nesting depth of enclosing loops — so a `break`/`continue` outside any loop is
    /// rejected cleanly rather than producing dangling jumps.
    pub(crate) loop_depth: usize,
    /// Type-parameter → concrete width for the instance being lowered (empty for a
    /// non-generic function).
    pub(crate) type_args: &'a HashMap<String, Width>,
    /// Const-parameter → concrete value for the instance being lowered (used as array
    /// lengths and as plain values; empty for a non-generic function).
    pub(crate) const_args: &'a HashMap<String, u16>,
    /// Shared monomorphization registry/worklist (calls register instances here).
    pub(crate) mono: &'a RefCell<Mono>,
}

impl Ctx<'_> {
    /// The width of a type annotation, resolving a generic parameter to its concrete
    /// width for this instantiation (`u8` → byte; a type-param → its bound width;
    /// anything else → word).
    pub(crate) fn width_of_type(&self, t: &syn::Type) -> Width {
        if let syn::Type::Path(p) = t {
            if let Some(id) = p.path.get_ident() {
                let s = id.to_string();
                if let Some(w) = self.type_args.get(&s) {
                    return *w;
                }
                if s == "u8" {
                    return Width::Byte;
                }
                if s == "u32" {
                    return Width::DWord;
                }
            }
        }
        Width::Word
    }

    /// A struct's field layout — a regular struct from the eager map, or a const-/
    /// generic struct *instance* (`Buf$8`) registered on demand at construction.
    pub(crate) fn struct_fields(&self, name: &str) -> Option<Vec<FieldDef>> {
        if let Some(f) = self.structs.get(name) {
            return Some(f.clone());
        }
        self.mono.borrow().struct_instances.get(name).cloned()
    }

    /// Evaluate an array length to a value — an integer literal, or a const-generic
    /// parameter resolved to this instance's value.
    pub(crate) fn eval_len(&self, e: &syn::Expr) -> Result<u16, String> {
        if let syn::Expr::Path(p) = e {
            if let Some(id) = p.path.get_ident() {
                if let Some(n) = self.const_args.get(&id.to_string()) {
                    return Ok(*n);
                }
            }
        }
        if let syn::Expr::Lit(l) = e {
            if let syn::Lit::Int(i) = &l.lit {
                return i.base10_parse::<u16>().map_err(|e| e.to_string());
            }
        }
        Err("array length must be an integer literal or a const-generic parameter".into())
    }
}

/// Lower every `fn` in a file to `(name, Func)`, using the file's struct layouts and
/// the caller's handle-routing config (empty for plain generic compilation).
pub fn lower_program(
    file: &syn::File,
    prelude: &PreludeConfig,
) -> Result<Vec<(String, Func)>, String> {
    let structs = collect_structs(file)?;
    let enums = collect_enums(file)?;
    let generic_structs = collect_generic_structs(file)?;
    let mut generic_fns = collect_generic_fns(file)?;
    collect_generic_methods(file, &generic_structs, &mut generic_fns)?;
    let mut mono_state = Mono::new(generic_fns);
    mono_state.generic_structs = generic_structs;
    let mono = RefCell::new(mono_state);
    let no_args = HashMap::new();
    let no_const = HashMap::new();
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            // `poke`/`peek` are host-only prelude intrinsics — skip their bodies.
            syn::Item::Fn(f) if is_intrinsic(&f.sig.ident.to_string()) => {}
            // Generic functions are lowered on demand, once per instantiation (below).
            syn::Item::Fn(f) if is_generic_fn(f) => {}
            syn::Item::Fn(f) => out.push((
                f.sig.ident.to_string(),
                lower_with(
                    f, &structs, &enums, prelude, &mono, &no_args, &no_const, None,
                )?,
            )),
            // `impl T { fn m(&mut self, …) }` — each method becomes a `T::m` function
            // taking `self` as a leading pointer argument.
            syn::Item::Impl(imp)
                if impl_is_for_generic_struct(imp, &mono.borrow().generic_structs) =>
            {
                // A const-generic struct's methods are instantiated per struct instance
                // (the worklist), not lowered here.
            }
            syn::Item::Impl(imp) => {
                let self_ty = type_name(&imp.self_ty)?;
                for it in &imp.items {
                    let syn::ImplItem::Fn(m) = it else {
                        return Err("only methods are supported in impl blocks".into());
                    };
                    if is_generic_sig(&m.sig) {
                        return Err("generic methods are not supported (use a free fn)".into());
                    }
                    let name = format!("{self_ty}::{}", m.sig.ident);
                    out.push((
                        name,
                        lower_method(
                            m, &self_ty, &structs, &enums, prelude, &mono, &no_args, &no_const,
                        )?,
                    ));
                }
            }
            syn::Item::Struct(_) | syn::Item::Enum(_) => {} // already collected
            syn::Item::Use(_) => {} // host-only imports — rustz80 has its own prelude
            other => {
                return Err(format!(
                    "only `fn`/`struct`/`enum`/`impl` items are supported: {other:?}"
                ))
            }
        }
    }

    // Drain the instantiation worklist: lowering each instance may request more
    // (a generic fn calling another), so loop until the queue is empty.
    loop {
        let inst = {
            let mut m = mono.borrow_mut();
            m.queue.pop()
        };
        let Some(inst) = inst else { break };
        let gf = mono.borrow().generics[&inst.generic].clone();
        // Split the instance's arguments into type-widths and const-values by the
        // matching parameter's kind.
        let mut type_args: HashMap<String, Width> = HashMap::new();
        let mut const_args: HashMap<String, u16> = HashMap::new();
        for (p, a) in gf.params.iter().zip(&inst.args) {
            match a {
                GArg::Width(w) => {
                    type_args.insert(p.name.clone(), *w);
                }
                GArg::Const(n) => {
                    const_args.insert(p.name.clone(), *n);
                }
            }
        }
        // A generic-struct method lowers with `self` typed as the matching struct
        // instance (`Buf$8`); a free fn has no `self`.
        let self_ty = gf
            .self_ty
            .as_ref()
            .map(|base| instance_name(base, &inst.args));
        let func = lower_with(
            &gf.item,
            &structs,
            &enums,
            prelude,
            &mono,
            &type_args,
            &const_args,
            self_ty.as_deref(),
        )?;
        out.push((inst.name, func));
    }

    if out.is_empty() {
        return Err("no functions found".into());
    }
    Ok(out)
}

/// Lower a standalone function (no struct/enum context — used by `compile_fn`).
pub fn lower(item: &syn::ItemFn) -> Result<Func, String> {
    let mono = RefCell::new(Mono::default());
    let no_args = HashMap::new();
    let no_const = HashMap::new();
    lower_with(
        item,
        &Structs::new(),
        &Enums::new(),
        &PreludeConfig::default(),
        &mono,
        &no_args,
        &no_const,
        None,
    )
}

/// Lower an `impl` method. The receiver (`&self`/`&mut self`) becomes a leading
/// pointer parameter; `self.field` reads/writes through it.
#[allow(clippy::too_many_arguments)]
fn lower_method<'a>(
    m: &syn::ImplItemFn,
    self_ty: &str,
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    mono: &'a RefCell<Mono>,
    type_args: &'a HashMap<String, Width>,
    const_args: &'a HashMap<String, u16>,
) -> Result<Func, String> {
    let mut ctx = new_ctx(structs, enums, prelude, mono, type_args, const_args);
    let params = lower_inputs(&m.sig.inputs, &mut ctx, Some(self_ty))?;
    let (body, ret) = lower_fn_block(&m.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_with<'a>(
    item: &syn::ItemFn,
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    mono: &'a RefCell<Mono>,
    type_args: &'a HashMap<String, Width>,
    const_args: &'a HashMap<String, u16>,
    self_ty: Option<&str>,
) -> Result<Func, String> {
    let mut ctx = new_ctx(structs, enums, prelude, mono, type_args, const_args);
    let params = lower_inputs(&item.sig.inputs, &mut ctx, self_ty)?;
    let (body, ret) = lower_fn_block(&item.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
}

#[allow(clippy::too_many_arguments)]
fn new_ctx<'a>(
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    mono: &'a RefCell<Mono>,
    type_args: &'a HashMap<String, Width>,
    const_args: &'a HashMap<String, u16>,
) -> Ctx<'a> {
    Ctx {
        vars: Vars::default(),
        structs,
        enums,
        prelude,
        temp: 0,
        loop_depth: 0,
        type_args,
        const_args,
        mono,
    }
}

/// Names the compiler handles itself (their host definitions are prelude-only).
fn is_intrinsic(name: &str) -> bool {
    matches!(name, "poke" | "peek" | "inport")
}

/// Declare a function's parameters, returning the count. `self_ty` is `Some` for
/// methods — then a leading `&self`/`&mut self` receiver is a pointer parameter.
fn lower_inputs(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
    ctx: &mut Ctx,
    self_ty: Option<&str>,
) -> Result<usize, String> {
    let mut params = 0;
    for (i, arg) in inputs.iter().enumerate() {
        match arg {
            syn::FnArg::Receiver(_) => {
                if i != 0 {
                    return Err("`self` must be the first parameter".into());
                }
                let sty = self_ty.ok_or("`self` outside an impl block")?;
                ctx.vars.declare_ptr("self", sty);
            }
            syn::FnArg::Typed(pt) => {
                let name = pat_ident(&pt.pat)?;
                match handle_type(&pt.ty, ctx.prelude) {
                    Some(h) => ctx.vars.declare_handle(&name, &h),
                    None => {
                        let w = ctx.width_of_type(&pt.ty);
                        ctx.vars.declare(&name, 1, None, w)
                    }
                };
            }
        }
        params += 1;
    }
    if params > 3 {
        return Err("more than 3 parameters not supported yet (no stack args)".into());
    }
    Ok(params)
}

/// Lower a function body: statements + an optional tail expression. The tail may be
/// a tuple `(a, b)` — a multi-value return placed in `HL`/`DE`/`BC`.
fn lower_fn_block(block: &syn::Block, ctx: &mut Ctx) -> Result<(Vec<Stmt>, Vec<Expr>), String> {
    let mut body = Vec::new();
    let mut ret = Vec::new();
    let stmts = &block.stmts;
    for (i, st) in stmts.iter().enumerate() {
        let last = i + 1 == stmts.len();
        match st {
            syn::Stmt::Local(local) => lower_local(local, ctx, &mut body)?,
            syn::Stmt::Expr(expr, semi) if last && semi.is_none() => match expr {
                syn::Expr::Tuple(t) => {
                    if t.elems.len() > 3 {
                        return Err("tuple returns support up to 3 values".into());
                    }
                    for e in &t.elems {
                        ret.push(lower_expr(e, ctx)?.0);
                    }
                }
                _ if is_value_expr(expr) => ret.push(lower_expr(expr, ctx)?.0),
                _ => lower_stmt_expr(expr, ctx, &mut body)?,
            },
            syn::Stmt::Expr(expr, _) => lower_stmt_expr(expr, ctx, &mut body)?,
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }
    Ok((body, ret))
}

fn is_value_expr(e: &syn::Expr) -> bool {
    matches!(
        e,
        syn::Expr::Lit(_)
            | syn::Expr::Path(_)
            | syn::Expr::Binary(_)
            | syn::Expr::Paren(_)
            | syn::Expr::Call(_)
            | syn::Expr::Index(_)
            | syn::Expr::Cast(_)
            | syn::Expr::Field(_)
            | syn::Expr::MethodCall(_)
    )
}

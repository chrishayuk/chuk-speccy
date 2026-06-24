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
use generics::{collect_generic_fns, is_generic_fn, is_generic_sig, Mono};
use layout::{collect_enums, collect_structs, type_name, Enums, Structs};
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
            }
        }
        Width::Word
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
    let mono = RefCell::new(Mono::new(collect_generic_fns(file)?));
    let no_args = HashMap::new();
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            // `poke`/`peek` are host-only prelude intrinsics — skip their bodies.
            syn::Item::Fn(f) if is_intrinsic(&f.sig.ident.to_string()) => {}
            // Generic functions are lowered on demand, once per instantiation (below).
            syn::Item::Fn(f) if is_generic_fn(f) => {}
            syn::Item::Fn(f) => out.push((
                f.sig.ident.to_string(),
                lower_with(f, &structs, &enums, prelude, &mono, &no_args)?,
            )),
            // `impl T { fn m(&mut self, …) }` — each method becomes a `T::m` function
            // taking `self` as a leading pointer argument.
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
                        lower_method(m, &self_ty, &structs, &enums, prelude, &mono, &no_args)?,
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
        let type_args: HashMap<String, Width> = gf
            .params
            .iter()
            .cloned()
            .zip(inst.type_args.iter().copied())
            .collect();
        let func = lower_with(&gf.item, &structs, &enums, prelude, &mono, &type_args)?;
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
    lower_with(
        item,
        &Structs::new(),
        &Enums::new(),
        &PreludeConfig::default(),
        &mono,
        &no_args,
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
) -> Result<Func, String> {
    let mut ctx = new_ctx(structs, enums, prelude, mono, type_args);
    let params = lower_inputs(&m.sig.inputs, &mut ctx, Some(self_ty))?;
    let (body, ret) = lower_fn_block(&m.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
}

fn lower_with<'a>(
    item: &syn::ItemFn,
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    mono: &'a RefCell<Mono>,
    type_args: &'a HashMap<String, Width>,
) -> Result<Func, String> {
    let mut ctx = new_ctx(structs, enums, prelude, mono, type_args);
    let params = lower_inputs(&item.sig.inputs, &mut ctx, None)?;
    let (body, ret) = lower_fn_block(&item.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
}

fn new_ctx<'a>(
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    mono: &'a RefCell<Mono>,
    type_args: &'a HashMap<String, Width>,
) -> Ctx<'a> {
    Ctx {
        vars: Vars::default(),
        structs,
        enums,
        prelude,
        temp: 0,
        loop_depth: 0,
        type_args,
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

/// Lower a function body: statements + an optional tail expression.
fn lower_fn_block(block: &syn::Block, ctx: &mut Ctx) -> Result<(Vec<Stmt>, Option<Expr>), String> {
    let mut body = Vec::new();
    let mut ret = None;
    let stmts = &block.stmts;
    for (i, st) in stmts.iter().enumerate() {
        let last = i + 1 == stmts.len();
        match st {
            syn::Stmt::Local(local) => lower_local(local, ctx, &mut body)?,
            syn::Stmt::Expr(expr, semi) => {
                if last && semi.is_none() && is_value_expr(expr) {
                    ret = Some(lower_expr(expr, ctx)?.0);
                } else {
                    lower_stmt_expr(expr, ctx, &mut body)?;
                }
            }
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

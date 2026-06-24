//! Statement and control-flow lowering: `let` bindings (scalars, arrays, struct
//! literals), assignment, `if`/`while`/`for`/`loop`/`match`, `break`/`continue`/
//! `return`, and the conditions they branch on. `for` desugars to a counted loop and
//! `match` to an if-chain over a scrutinee temp — no codegen support needed for either.

use super::expr::{
    array_base, lower_expr, lower_field_store, lower_index_store, lower_method_call, path_ident,
    path_str,
};
use super::generics::infer_struct_args;
use super::layout::{
    elem_width, field_offset, lit_len, member_name, resolve_enum_path, struct_slots, FieldDef,
};
use super::Ctx;
use crate::ir::*;

/// Resolve a struct literal to its concrete `(name, field layout)` — a regular struct,
/// or a const-/generic struct *instance* whose arguments are inferred from the literal
/// (registering the instance + its methods on first use).
fn resolve_struct_literal(
    lit: &syn::ExprStruct,
    ctx: &mut Ctx,
) -> Result<(String, Vec<FieldDef>), String> {
    let sbase = path_str(&lit.path)?;
    if let Some(f) = ctx.structs.get(&sbase) {
        return Ok((sbase, f.clone()));
    }
    let gs = ctx.mono.borrow().generic_structs.get(&sbase).cloned();
    let Some(gs) = gs else {
        return Err(format!("unknown struct {sbase}"));
    };
    let args = infer_struct_args(&gs, lit, ctx)?;
    let mangled = ctx.mono.borrow_mut().instantiate_struct(&sbase, args)?;
    let fields = ctx.mono.borrow().struct_instances[&mangled].clone();
    Ok((mangled, fields))
}

/// An array length — an integer literal, or a const-generic parameter resolved to its
/// instance value (`let a = [0u16; N]` inside `fn f<const N: usize>()`).
fn array_len(e: &syn::Expr, ctx: &Ctx) -> Result<usize, String> {
    if let syn::Expr::Path(p) = e {
        if let Some(id) = p.path.get_ident() {
            if let Some(n) = ctx.const_args.get(&id.to_string()) {
                return Ok(*n as usize);
            }
        }
    }
    lit_len(e)
}

pub(crate) fn lower_local(
    local: &syn::Local,
    ctx: &mut Ctx,
    body: &mut Vec<Stmt>,
) -> Result<(), String> {
    let init = local.init.as_ref().ok_or("`let` needs an initializer")?;
    // `let (a, b) = …` — a tuple destructure (a tuple literal or a multi-value return).
    if let syn::Pat::Tuple(pt) = &local.pat {
        return lower_tuple_let(pt, &init.expr, ctx, body);
    }
    let name = pat_ident(&local.pat)?;
    match &*init.expr {
        // `[Cell { … }; N]` — a struct-element array; each element is `Cell`'s slots.
        syn::Expr::Repeat(r) if matches!(&*r.expr, syn::Expr::Struct(_)) => {
            let syn::Expr::Struct(slit) = &*r.expr else {
                unreachable!()
            };
            let n = array_len(&r.len, ctx)?;
            let elem_name = path_str(&slit.path)?;
            let efields = ctx
                .struct_fields(&elem_name)
                .ok_or_else(|| format!("unknown struct {elem_name}"))?;
            let stride = struct_slots(&efields);
            let base = ctx.vars.declare_struct_array(&name, n * stride, &elem_name);
            for i in 0..n {
                for fv in &slit.fields {
                    let foff = field_offset(&efields, &member_name(&fv.member)?)?;
                    let v = lower_expr(&fv.expr, ctx)?.0;
                    body.push(Stmt::Assign(base + i * stride + foff, v));
                }
            }
        }
        syn::Expr::Repeat(r) => {
            let n = array_len(&r.len, ctx)?;
            let elem = elem_width(&r.expr);
            let base = ctx.vars.declare(&name, n, None, elem);
            for i in 0..n {
                let v = lower_expr(&r.expr, ctx)?.0;
                body.push(Stmt::StoreIndex(base, Expr::Lit(i as u16), v, elem));
            }
        }
        syn::Expr::Array(arr) => {
            let elem = arr.elems.first().map(elem_width).unwrap_or(Width::Word);
            let base = ctx.vars.declare(&name, arr.elems.len(), None, elem);
            for (i, e) in arr.elems.iter().enumerate() {
                let v = lower_expr(e, ctx)?.0;
                body.push(Stmt::StoreIndex(base, Expr::Lit(i as u16), v, elem));
            }
        }
        syn::Expr::Struct(lit) => {
            // Resolve to a concrete struct: a regular struct, or a const-/generic
            // struct *instance* (`Buf$8`) inferred from this literal.
            let (sname, fields) = resolve_struct_literal(lit, ctx)?;
            let base = ctx.vars.declare(
                &name,
                struct_slots(&fields),
                Some(sname.clone()),
                Width::Word,
            );
            for fv in &lit.fields {
                let fname = member_name(&fv.member)?;
                let off = field_offset(&fields, &fname)?;
                let fd = fields.iter().find(|f| f.name == fname);
                let slots = fd.map_or(1, |f| f.slots);
                // A `[Cell; N]` struct-element field — `[Cell { … }; N]` or `[c0, c1, …]`;
                // store each element's sub-fields.
                if let Some(es) = fd.and_then(|f| f.elem_struct.clone()) {
                    let efields = ctx
                        .struct_fields(&es)
                        .ok_or_else(|| format!("unknown struct {es}"))?;
                    let esize = struct_slots(&efields).max(1);
                    let elems: Vec<&syn::Expr> = match &fv.expr {
                        syn::Expr::Repeat(r) => vec![&*r.expr; slots / esize],
                        syn::Expr::Array(arr) => arr.elems.iter().collect(),
                        _ => {
                            return Err(format!(
                                "array field `{fname}` must be `[{es} {{ … }}; N]` or `[…]`"
                            ))
                        }
                    };
                    for (e, ev) in elems.iter().enumerate() {
                        let syn::Expr::Struct(slit) = ev else {
                            return Err(format!("element of `{fname}` must be a `{es}` literal"));
                        };
                        for fv2 in &slit.fields {
                            let foff = field_offset(&efields, &member_name(&fv2.member)?)?;
                            let v = lower_expr(&fv2.expr, ctx)?.0;
                            body.push(Stmt::Assign(base + off + e * esize + foff, v));
                        }
                    }
                    continue;
                }
                match &fv.expr {
                    // A tuple field is initialised by a tuple literal — one value per slot.
                    syn::Expr::Tuple(t) => {
                        if t.elems.len() != slots {
                            return Err(format!("tuple field `{fname}` expects {slots} values"));
                        }
                        for (i, e) in t.elems.iter().enumerate() {
                            let v = lower_expr(e, ctx)?.0;
                            body.push(Stmt::Assign(base + off + i, v));
                        }
                    }
                    // An array field initialised `[v; N]` — fill its `slots` slots.
                    syn::Expr::Repeat(r) => {
                        for i in 0..slots {
                            let v = lower_expr(&r.expr, ctx)?.0;
                            body.push(Stmt::Assign(base + off + i, v));
                        }
                    }
                    // An array field initialised `[e0, e1, …]`.
                    syn::Expr::Array(arr) => {
                        if arr.elems.len() != slots {
                            return Err(format!("array field `{fname}` expects {slots} values"));
                        }
                        for (i, e) in arr.elems.iter().enumerate() {
                            let v = lower_expr(e, ctx)?.0;
                            body.push(Stmt::Assign(base + off + i, v));
                        }
                    }
                    _ if slots == 1 => {
                        let v = lower_expr(&fv.expr, ctx)?.0;
                        body.push(Stmt::Assign(base + off, v));
                    }
                    _ => return Err(format!("field `{fname}` expects {slots} values")),
                }
            }
        }
        other => {
            let (e, ty) = lower_expr(other, ctx)?;
            let base = ctx.vars.declare(&name, 1, None, ty);
            body.push(Stmt::Assign(base, e));
        }
    }
    Ok(())
}

/// Lower `let (a, b, …) = init`. The RHS is either a tuple literal (each component
/// assigned to its own slot) or a function call returning a tuple (one
/// [`Stmt::AssignTuple`] distributing `HL`/`DE`/`BC` into the slots).
fn lower_tuple_let(
    pt: &syn::PatTuple,
    init: &syn::Expr,
    ctx: &mut Ctx,
    body: &mut Vec<Stmt>,
) -> Result<(), String> {
    let names: Vec<String> = pt.elems.iter().map(pat_ident).collect::<Result<_, _>>()?;
    if names.len() > 3 {
        return Err("tuple bindings support up to 3 values".into());
    }
    match init {
        syn::Expr::Tuple(t) => {
            if t.elems.len() != names.len() {
                return Err("tuple binding has the wrong number of values".into());
            }
            // Evaluate all components before binding (Rust evaluates the RHS first).
            let vals: Vec<(Expr, Width)> = t
                .elems
                .iter()
                .map(|e| lower_expr(e, ctx))
                .collect::<Result<_, _>>()?;
            for (name, (v, ty)) in names.iter().zip(vals) {
                let base = ctx.vars.declare(name, 1, None, ty);
                body.push(Stmt::Assign(base, v));
            }
        }
        call => {
            let (e, _) = lower_expr(call, ctx)?;
            if !matches!(e, Expr::Call(..)) {
                return Err("a tuple binding needs a tuple literal or a function call".into());
            }
            let slots: Vec<usize> = names
                .iter()
                .map(|n| ctx.vars.declare(n, 1, None, Width::Word))
                .collect();
            body.push(Stmt::AssignTuple(slots, e));
        }
    }
    Ok(())
}

/// Lower `arr[i] = rhs`. For a struct-element array assigned a struct literal
/// (`a[i] = Cell { x, y }`), store each field at the element's computed address;
/// otherwise it's an ordinary scalar/field array store.
fn lower_index_assign(
    ix: &syn::ExprIndex,
    rhs: &syn::Expr,
    ctx: &mut Ctx,
    body: &mut Vec<Stmt>,
) -> Result<(), String> {
    // A struct-element array (local `a` or struct field `self.cells`) assigned a struct
    // literal: store each field at the element's computed address.
    if let Ok((base_addr, elem_struct)) = array_base(&ix.expr, ctx) {
        let syn::Expr::Struct(slit) = rhs else {
            return Err(format!(
                "assign a struct-array element with a struct literal (`{elem_struct} {{ … }}`)"
            ));
        };
        let efields = ctx
            .struct_fields(&elem_struct)
            .ok_or_else(|| format!("unknown struct {elem_struct}"))?;
        let stride = (struct_slots(&efields) * 2) as u16;
        let idx = lower_expr(&ix.index, ctx)?.0;
        for fv in &slit.fields {
            let foff = field_offset(&efields, &member_name(&fv.member)?)?;
            let v = lower_expr(&fv.expr, ctx)?.0;
            let elem = Expr::Bin(
                BinOp::Add,
                Box::new(base_addr.clone()),
                Box::new(Expr::MulConst(Box::new(idx.clone()), stride)),
                Width::Word,
            );
            let addr = if foff == 0 {
                elem
            } else {
                Expr::Bin(
                    BinOp::Add,
                    Box::new(elem),
                    Box::new(Expr::Lit((foff * 2) as u16)),
                    Width::Word,
                )
            };
            body.push(Stmt::StoreAt(addr, v, Width::Word));
        }
        return Ok(());
    }
    body.push(lower_index_store(ix, rhs, ctx)?);
    Ok(())
}

pub(crate) fn lower_stmt_expr(
    expr: &syn::Expr,
    ctx: &mut Ctx,
    body: &mut Vec<Stmt>,
) -> Result<(), String> {
    match expr {
        syn::Expr::Assign(a) => match &*a.left {
            syn::Expr::Index(ix) => lower_index_assign(ix, &a.right, ctx, body)?,
            syn::Expr::Field(f) => {
                let val = lower_expr(&a.right, ctx)?.0;
                body.push(lower_field_store(f, val, ctx)?);
            }
            _ => {
                let slot = ctx.vars.base(&path_ident(&a.left)?);
                let e = lower_expr(&a.right, ctx)?.0;
                body.push(Stmt::Assign(slot, e));
            }
        },
        syn::Expr::If(ifx) => {
            let cond = lower_cond(&ifx.cond, ctx)?;
            let then = lower_block(&ifx.then_branch, ctx)?;
            let els = match &ifx.else_branch {
                Some((_, e)) => lower_else(e, ctx)?,
                None => Vec::new(),
            };
            body.push(Stmt::If(cond, then, els));
        }
        syn::Expr::While(w) => {
            let cond = lower_cond(&w.cond, ctx)?;
            ctx.loop_depth += 1;
            let inner = lower_block(&w.body, ctx)?;
            ctx.loop_depth -= 1;
            body.push(Stmt::While(cond, inner));
        }
        // `match` lowers to an if-chain over a scrutinee temporary (no codegen change).
        syn::Expr::Match(m) => {
            let scrut = lower_expr(&m.expr, ctx)?.0;
            let temp = ctx
                .vars
                .declare(&format!("__match{}", ctx.temp), 1, None, Width::Word);
            ctx.temp += 1;
            body.push(Stmt::Assign(temp, scrut));

            let mut default: Vec<Stmt> = Vec::new();
            let mut arms: Vec<(Expr, Vec<Stmt>)> = Vec::new();
            for arm in &m.arms {
                let arm_body = lower_arm_body(&arm.body, ctx)?;
                match pattern_value(&arm.pat, ctx)? {
                    Some(v) => arms.push((v, arm_body)),
                    None => default = arm_body, // `_` wildcard
                }
            }
            let mut chain = default;
            for (val, arm_body) in arms.into_iter().rev() {
                let cond = Cond {
                    cmp: Cmp::Eq,
                    lhs: Expr::Var(temp),
                    rhs: val,
                };
                chain = vec![Stmt::If(cond, arm_body, chain)];
            }
            body.extend(chain);
        }
        // A call as a statement: the `poke` intrinsic, or a void call (discarded).
        syn::Expr::Call(c) => {
            let name = path_ident(&c.func)?;
            if name == "poke" {
                let addr = c.args.first().ok_or("poke(addr, val) needs an address")?;
                let val = c.args.get(1).ok_or("poke(addr, val) needs a value")?;
                let addr = lower_expr(addr, ctx)?.0;
                let val = lower_expr(val, ctx)?.0;
                body.push(Stmt::Poke(addr, val));
            } else {
                body.push(Stmt::Eval(lower_expr(expr, ctx)?.0));
            }
        }
        // A method call as a statement (e.g. `self.move_head();`).
        syn::Expr::MethodCall(m) => {
            body.push(Stmt::Eval(lower_method_call(m, ctx)?.0));
        }
        // `for var in a..b { … }` — desugared to an init + a counted loop.
        syn::Expr::ForLoop(fl) => lower_for(fl, ctx, body)?,
        // `loop { … }` — an unconditional loop (exit via `break`/`return`).
        syn::Expr::Loop(l) => {
            if l.label.is_some() {
                return Err("loop labels are not supported".into());
            }
            ctx.loop_depth += 1;
            let inner = lower_block(&l.body, ctx)?;
            ctx.loop_depth -= 1;
            body.push(Stmt::Loop(inner));
        }
        syn::Expr::Break(b) => {
            if b.expr.is_some() {
                return Err("`break <value>` is not supported".into());
            }
            if b.label.is_some() {
                return Err("labeled `break` is not supported".into());
            }
            if ctx.loop_depth == 0 {
                return Err("`break` outside a loop".into());
            }
            body.push(Stmt::Break);
        }
        syn::Expr::Continue(c) => {
            if c.label.is_some() {
                return Err("labeled `continue` is not supported".into());
            }
            if ctx.loop_depth == 0 {
                return Err("`continue` outside a loop".into());
            }
            body.push(Stmt::Continue);
        }
        syn::Expr::Return(r) => {
            let val = match &r.expr {
                Some(e) => Some(lower_expr(e, ctx)?.0),
                None => None,
            };
            body.push(Stmt::Return(val));
        }
        other => return Err(format!("unsupported statement expression: {other:?}")),
    }
    Ok(())
}

/// Lower `for var in start..end { body }` to: assign the loop variable to `start`,
/// snapshot the (once-evaluated) `end` bound into a temp, and emit a [`Stmt::ForRange`]
/// whose step (`var += 1`) is the `continue` target. The loop variable's width is
/// inferred from the start bound.
fn lower_for(fl: &syn::ExprForLoop, ctx: &mut Ctx, body: &mut Vec<Stmt>) -> Result<(), String> {
    if fl.label.is_some() {
        return Err("loop labels are not supported".into());
    }
    // `for _ in …` still needs a counter slot — synthesise a hidden name for it.
    let var_name = match &*fl.pat {
        syn::Pat::Wild(_) => {
            let n = format!("__foridx{}", ctx.temp);
            ctx.temp += 1;
            n
        }
        p => pat_ident(p)?,
    };
    let syn::Expr::Range(range) = &*fl.expr else {
        return Err("`for` only supports integer ranges (`a..b` / `a..=b`)".into());
    };
    let start = range
        .start
        .as_ref()
        .ok_or("`for` range needs a start bound")?;
    let end_expr = range.end.as_ref().ok_or("`for` range needs an end bound")?;
    let inclusive = matches!(range.limits, syn::RangeLimits::Closed(_));

    // Evaluate both bounds before declaring the loop variable (they cannot see it).
    let (start_e, width) = lower_expr(start, ctx)?;
    let (end_e, _) = lower_expr(end_expr, ctx)?;
    let end_temp = ctx
        .vars
        .declare(&format!("__forend{}", ctx.temp), 1, None, width);
    ctx.temp += 1;
    let var = ctx.vars.declare(&var_name, 1, None, width);

    body.push(Stmt::Assign(var, start_e));
    body.push(Stmt::Assign(end_temp, end_e));

    ctx.loop_depth += 1;
    let inner = lower_block(&fl.body, ctx)?;
    ctx.loop_depth -= 1;

    body.push(Stmt::ForRange {
        var,
        end: Expr::Var(end_temp),
        inclusive,
        width,
        body: inner,
    });
    Ok(())
}

fn lower_else(e: &syn::Expr, ctx: &mut Ctx) -> Result<Vec<Stmt>, String> {
    match e {
        syn::Expr::Block(b) => lower_block(&b.block, ctx),
        syn::Expr::If(_) => {
            let mut v = Vec::new();
            lower_stmt_expr(e, ctx, &mut v)?;
            Ok(v)
        }
        other => Err(format!("unsupported else branch: {other:?}")),
    }
}

fn lower_block(b: &syn::Block, ctx: &mut Ctx) -> Result<Vec<Stmt>, String> {
    let mut body = Vec::new();
    for st in &b.stmts {
        match st {
            syn::Stmt::Local(local) => lower_local(local, ctx, &mut body)?,
            syn::Stmt::Expr(expr, _) => lower_stmt_expr(expr, ctx, &mut body)?,
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }
    Ok(body)
}

fn lower_cond(expr: &syn::Expr, ctx: &mut Ctx) -> Result<Cond, String> {
    // A comparison maps directly; any other bool expression means "is non-zero"
    // (e.g. `if input.held(Button::Left)`).
    if let syn::Expr::Binary(b) = expr {
        if let Some(cmp) = cmp_op(&b.op) {
            return Ok(Cond {
                cmp,
                lhs: lower_expr(&b.left, ctx)?.0,
                rhs: lower_expr(&b.right, ctx)?.0,
            });
        }
    }
    if let syn::Expr::Paren(p) = expr {
        return lower_cond(&p.expr, ctx);
    }
    let (e, _) = lower_expr(expr, ctx)?;
    Ok(Cond {
        cmp: Cmp::Ne,
        lhs: e,
        rhs: Expr::Lit(0),
    })
}

fn cmp_op(op: &syn::BinOp) -> Option<Cmp> {
    Some(match op {
        syn::BinOp::Lt(_) => Cmp::Lt,
        syn::BinOp::Le(_) => Cmp::Le,
        syn::BinOp::Gt(_) => Cmp::Gt,
        syn::BinOp::Ge(_) => Cmp::Ge,
        syn::BinOp::Eq(_) => Cmp::Eq,
        syn::BinOp::Ne(_) => Cmp::Ne,
        _ => return None,
    })
}

/// A match arm body: a `{ block }` or a single expression-statement.
fn lower_arm_body(e: &syn::Expr, ctx: &mut Ctx) -> Result<Vec<Stmt>, String> {
    match e {
        syn::Expr::Block(b) => lower_block(&b.block, ctx),
        other => {
            let mut v = Vec::new();
            lower_stmt_expr(other, ctx, &mut v)?;
            Ok(v)
        }
    }
}

/// A match pattern's value, or `None` for the `_` wildcard.
fn pattern_value(pat: &syn::Pat, ctx: &Ctx) -> Result<Option<Expr>, String> {
    match pat {
        syn::Pat::Wild(_) => Ok(None),
        syn::Pat::Lit(pl) => match &pl.lit {
            syn::Lit::Int(i) => Ok(Some(Expr::Lit(
                i.base10_parse::<u16>().map_err(|e| e.to_string())?,
            ))),
            other => Err(format!("only integer literal patterns: {other:?}")),
        },
        syn::Pat::Path(pp) => resolve_enum_path(&pp.path, ctx.enums)
            .map(|v| Some(Expr::Lit(v)))
            .ok_or_else(|| "unknown enum variant in pattern".into()),
        other => Err(format!("unsupported match pattern: {other:?}")),
    }
}

pub(crate) fn pat_ident(pat: &syn::Pat) -> Result<String, String> {
    match pat {
        syn::Pat::Ident(p) => Ok(p.ident.to_string()),
        syn::Pat::Type(t) => pat_ident(&t.pat),
        other => Err(format!("unsupported let pattern: {other:?}")),
    }
}

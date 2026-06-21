//! Lower a `syn` `ItemFn` (accepted subset) to the IR. Unsupported nodes become
//! errors — the "not supported on Z80 / host-only" signal.
//!
//! Structs are a lowering-only concern: every field has a constant offset, so
//! `s.field` lowers to a plain slot access — codegen needs no struct awareness.

use crate::ir::*;
use std::collections::HashMap;

/// Struct layouts: name → field names in declaration order (offset = position;
/// every field is one `u16` slot in Stage 1).
type Structs = HashMap<String, Vec<String>>;

/// C-like enum layouts: name → variant names in order (value = position).
type Enums = HashMap<String, Vec<String>>;

/// Per-function lowering context: locals + the program's struct/enum layouts.
struct Ctx<'a> {
    vars: Vars,
    structs: &'a Structs,
    enums: &'a Enums,
    /// Counter for synthesised `match` scrutinee temporaries.
    temp: usize,
}

struct VarInfo {
    base: usize,
    sty: Option<String>,
    ty: Width, // scalar value type, or array element type
}

/// Name → variable info. Flat scoping; arrays use one 2-byte slot per element.
#[derive(Default)]
struct Vars {
    map: HashMap<String, VarInfo>,
    next: usize,
}

impl Vars {
    fn declare(&mut self, name: &str, size: usize, sty: Option<String>, ty: Width) -> usize {
        let base = self.next;
        self.map.insert(name.to_string(), VarInfo { base, sty, ty });
        self.next += size;
        base
    }
    fn base(&mut self, name: &str) -> usize {
        match self.map.get(name) {
            Some(v) => v.base,
            None => self.declare(name, 1, None, Width::Word),
        }
    }
    fn struct_of(&self, name: &str) -> Option<(usize, String)> {
        self.map.get(name).and_then(|v| v.sty.as_ref().map(|s| (v.base, s.clone())))
    }
    /// The variable's value type (scalar) or element type (array).
    fn ty(&self, name: &str) -> Width {
        self.map.get(name).map(|v| v.ty).unwrap_or(Width::Word)
    }
}

/// Lower every `fn` in a file to `(name, Func)`, using the file's struct layouts.
pub fn lower_program(file: &syn::File) -> Result<Vec<(String, Func)>, String> {
    let structs = collect_structs(file)?;
    let enums = collect_enums(file);
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(f) => out.push((f.sig.ident.to_string(), lower_with(f, &structs, &enums)?)),
            syn::Item::Struct(_) | syn::Item::Enum(_) => {} // already collected
            other => return Err(format!("only `fn`/`struct`/`enum` items are supported: {other:?}")),
        }
    }
    if out.is_empty() {
        return Err("no functions found".into());
    }
    Ok(out)
}

/// Lower a standalone function (no struct/enum context — used by `compile_fn`).
pub fn lower(item: &syn::ItemFn) -> Result<Func, String> {
    lower_with(item, &Structs::new(), &Enums::new())
}

fn collect_enums(file: &syn::File) -> Enums {
    let mut m = Enums::new();
    for item in &file.items {
        if let syn::Item::Enum(e) = item {
            let variants = e.variants.iter().map(|v| v.ident.to_string()).collect();
            m.insert(e.ident.to_string(), variants);
        }
    }
    m
}

fn collect_structs(file: &syn::File) -> Result<Structs, String> {
    let mut m = Structs::new();
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            let syn::Fields::Named(named) = &s.fields else {
                return Err(format!("only named-field structs are supported: {}", s.ident));
            };
            let mut fields = Vec::new();
            for f in &named.named {
                // Stage 1c: scalar fields only (each is one slot). Array/nested
                // fields would mislay offsets, so reject them clearly.
                if !matches!(f.ty, syn::Type::Path(_)) {
                    return Err(format!("only scalar struct fields are supported: {}", s.ident));
                }
                fields.push(f.ident.as_ref().unwrap().to_string());
            }
            m.insert(s.ident.to_string(), fields);
        }
    }
    Ok(m)
}

fn lower_with(item: &syn::ItemFn, structs: &Structs, enums: &Enums) -> Result<Func, String> {
    let mut ctx = Ctx { vars: Vars::default(), structs, enums, temp: 0 };
    let mut body = Vec::new();
    let mut ret = None;

    let mut params = 0;
    for arg in &item.sig.inputs {
        let syn::FnArg::Typed(pt) = arg else {
            return Err("`self` parameters are not supported".into());
        };
        ctx.vars.declare(&pat_ident(&pt.pat)?, 1, None, ty_of_type(&pt.ty));
        params += 1;
    }
    if params > 3 {
        return Err("more than 3 parameters not supported yet (no stack args)".into());
    }

    let stmts = &item.block.stmts;
    for (i, st) in stmts.iter().enumerate() {
        let last = i + 1 == stmts.len();
        match st {
            syn::Stmt::Local(local) => lower_local(local, &mut ctx, &mut body)?,
            syn::Stmt::Expr(expr, semi) => {
                if last && semi.is_none() && is_value_expr(expr) {
                    ret = Some(lower_expr(expr, &mut ctx)?.0);
                } else {
                    lower_stmt_expr(expr, &mut ctx, &mut body)?;
                }
            }
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }

    Ok(Func { params, n_locals: ctx.vars.next, body, ret })
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
    )
}

fn lower_local(local: &syn::Local, ctx: &mut Ctx, body: &mut Vec<Stmt>) -> Result<(), String> {
    let init = local.init.as_ref().ok_or("`let` needs an initializer")?;
    let name = pat_ident(&local.pat)?;
    match &*init.expr {
        syn::Expr::Repeat(r) => {
            let n = lit_len(&r.len)?;
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
            let sname = path_str(&lit.path)?;
            let fields = ctx.structs.get(&sname).ok_or_else(|| format!("unknown struct {sname}"))?.clone();
            let base = ctx.vars.declare(&name, fields.len(), Some(sname.clone()), Width::Word);
            for fv in &lit.fields {
                let fname = member_name(&fv.member)?;
                let off = field_offset(&fields, &fname)?;
                let v = lower_expr(&fv.expr, ctx)?.0;
                body.push(Stmt::Assign(base + off, v));
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

fn lower_stmt_expr(expr: &syn::Expr, ctx: &mut Ctx, body: &mut Vec<Stmt>) -> Result<(), String> {
    match expr {
        syn::Expr::Assign(a) => match &*a.left {
            syn::Expr::Index(ix) => {
                let arr = path_ident(&ix.expr)?;
                let base = ctx.vars.base(&arr);
                let w = ctx.vars.ty(&arr);
                let idx = lower_expr(&ix.index, ctx)?.0;
                let val = lower_expr(&a.right, ctx)?.0;
                body.push(Stmt::StoreIndex(base, idx, val, w));
            }
            syn::Expr::Field(_) => {
                let slot = field_slot(&a.left, ctx)?;
                let e = lower_expr(&a.right, ctx)?.0;
                body.push(Stmt::Assign(slot, e));
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
            let inner = lower_block(&w.body, ctx)?;
            body.push(Stmt::While(cond, inner));
        }
        // `match` lowers to an if-chain over a scrutinee temporary (no codegen change).
        syn::Expr::Match(m) => {
            let scrut = lower_expr(&m.expr, ctx)?.0;
            let temp = ctx.vars.declare(&format!("__match{}", ctx.temp), 1, None, Width::Word);
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
                let cond = Cond { cmp: Cmp::Eq, lhs: Expr::Var(temp), rhs: val };
                chain = vec![Stmt::If(cond, arm_body, chain)];
            }
            body.extend(chain);
        }
        other => return Err(format!("unsupported statement expression: {other:?}")),
    }
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
    let syn::Expr::Binary(b) = expr else {
        return Err(format!("condition must be a comparison, got {expr:?}"));
    };
    let cmp = match b.op {
        syn::BinOp::Lt(_) => Cmp::Lt,
        syn::BinOp::Le(_) => Cmp::Le,
        syn::BinOp::Gt(_) => Cmp::Gt,
        syn::BinOp::Ge(_) => Cmp::Ge,
        syn::BinOp::Eq(_) => Cmp::Eq,
        syn::BinOp::Ne(_) => Cmp::Ne,
        other => return Err(format!("unsupported comparison op: {other:?}")),
    };
    Ok(Cond { cmp, lhs: lower_expr(&b.left, ctx)?.0, rhs: lower_expr(&b.right, ctx)?.0 })
}

/// Lower an expression, returning its IR and inferred width (`u8`/`u16`).
fn lower_expr(expr: &syn::Expr, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
    match expr {
        syn::Expr::Lit(l) => {
            let syn::Lit::Int(i) = &l.lit else {
                return Err("only integer literals are supported".into());
            };
            let w = if i.suffix() == "u8" { Width::Byte } else { Width::Word };
            Ok((Expr::Lit(i.base10_parse::<u16>().map_err(|e| e.to_string())?), w))
        }
        syn::Expr::Path(p) => match resolve_enum_path(&p.path, ctx.enums) {
            Some(v) => Ok((Expr::Lit(v), Width::Word)),
            None => {
                let name = path_ident(expr)?;
                Ok((Expr::Var(ctx.vars.base(&name)), ctx.vars.ty(&name)))
            }
        },
        syn::Expr::Paren(p) => lower_expr(&p.expr, ctx),
        // `e as u8` truncates to a byte; `as u16`/`as usize` is a no-op (16-bit).
        syn::Expr::Cast(c) => {
            let (e, _) = lower_expr(&c.expr, ctx)?;
            if ty_of_type(&c.ty) == Width::Byte {
                Ok((Expr::Trunc(Box::new(e)), Width::Byte))
            } else {
                Ok((e, Width::Word))
            }
        }
        syn::Expr::Field(_) => Ok((Expr::Var(field_slot(expr, ctx)?), Width::Word)),
        syn::Expr::Index(ix) => {
            let arr = path_ident(&ix.expr)?;
            let base = ctx.vars.base(&arr);
            let w = ctx.vars.ty(&arr);
            let idx = lower_expr(&ix.index, ctx)?.0;
            Ok((Expr::Index(base, Box::new(idx), w), w))
        }
        syn::Expr::Binary(b) => {
            let op = bin_op(&b.op)?;
            let (le, lw) = lower_expr(&b.left, ctx)?;
            let (re, _) = lower_expr(&b.right, ctx)?;
            Ok((Expr::Bin(op, Box::new(le), Box::new(re), lw), lw))
        }
        // `x.wrapping_add(y)` etc. — the op at the receiver's width.
        syn::Expr::MethodCall(m) => {
            let op = match m.method.to_string().as_str() {
                "wrapping_add" => BinOp::Add,
                "wrapping_sub" => BinOp::Sub,
                "wrapping_mul" => BinOp::Mul,
                other => return Err(format!("unsupported method: {other}")),
            };
            let (recv, rw) = lower_expr(&m.receiver, ctx)?;
            let arg = m.args.first().ok_or("wrapping_* needs an argument")?;
            let (re, _) = lower_expr(arg, ctx)?;
            Ok((Expr::Bin(op, Box::new(recv), Box::new(re), rw), rw))
        }
        syn::Expr::Call(c) => {
            let name = path_ident(&c.func)?;
            if c.args.len() > 3 {
                return Err("more than 3 call arguments not supported yet".into());
            }
            let args = c.args.iter().map(|a| Ok(lower_expr(a, ctx)?.0)).collect::<Result<_, String>>()?;
            Ok((Expr::Call(name, args), Width::Word)) // Stage 1f assumes u16 returns
        }
        other => Err(format!("unsupported expression: {other:?}")),
    }
}

fn bin_op(op: &syn::BinOp) -> Result<BinOp, String> {
    Ok(match op {
        syn::BinOp::Add(_) => BinOp::Add,
        syn::BinOp::Sub(_) => BinOp::Sub,
        syn::BinOp::Mul(_) => BinOp::Mul,
        syn::BinOp::Div(_) => BinOp::Div,
        syn::BinOp::Rem(_) => BinOp::Rem,
        other => return Err(format!("unsupported arithmetic op: {other:?}")),
    })
}

/// A type annotation's width (`u8` → byte, everything else → word).
fn ty_of_type(t: &syn::Type) -> Width {
    if let syn::Type::Path(p) = t {
        if p.path.is_ident("u8") {
            return Width::Byte;
        }
    }
    Width::Word
}

/// Resolve `s.field` (a `syn::Expr::Field`) to its constant local slot.
fn field_slot(expr: &syn::Expr, ctx: &mut Ctx) -> Result<usize, String> {
    let syn::Expr::Field(f) = expr else {
        return Err("expected a field access".into());
    };
    let obj = path_ident(&f.base)?;
    let (base, sname) = ctx.vars.struct_of(&obj).ok_or_else(|| format!("{obj} is not a struct"))?;
    let fields = ctx.structs.get(&sname).ok_or_else(|| format!("unknown struct {sname}"))?;
    Ok(base + field_offset(fields, &member_name(&f.member)?)?)
}

/// `Enum::Variant` (a 2-segment path) → its integer value, if known.
fn resolve_enum_path(path: &syn::Path, enums: &Enums) -> Option<u16> {
    if path.segments.len() != 2 {
        return None;
    }
    let name = path.segments[0].ident.to_string();
    let variant = path.segments[1].ident.to_string();
    enums.get(&name)?.iter().position(|v| *v == variant).map(|i| i as u16)
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
            syn::Lit::Int(i) => Ok(Some(Expr::Lit(i.base10_parse::<u16>().map_err(|e| e.to_string())?))),
            other => Err(format!("only integer literal patterns: {other:?}")),
        },
        syn::Pat::Path(pp) => resolve_enum_path(&pp.path, ctx.enums)
            .map(|v| Some(Expr::Lit(v)))
            .ok_or_else(|| "unknown enum variant in pattern".into()),
        other => Err(format!("unsupported match pattern: {other:?}")),
    }
}

fn field_offset(fields: &[String], name: &str) -> Result<usize, String> {
    fields.iter().position(|f| f == name).ok_or_else(|| format!("no field {name}"))
}

fn member_name(m: &syn::Member) -> Result<String, String> {
    match m {
        syn::Member::Named(n) => Ok(n.to_string()),
        syn::Member::Unnamed(_) => Err("tuple-struct fields not supported".into()),
    }
}

/// Element width inferred from an initialiser value's literal suffix (`0u8` →
/// byte; everything else → word). Good enough for Stage 1e byte arrays.
fn elem_width(e: &syn::Expr) -> Width {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            if i.suffix() == "u8" {
                return Width::Byte;
            }
        }
    }
    Width::Word
}

fn lit_len(e: &syn::Expr) -> Result<usize, String> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<usize>().map_err(|e| e.to_string());
        }
    }
    Err("array length must be an integer literal".into())
}

fn pat_ident(pat: &syn::Pat) -> Result<String, String> {
    match pat {
        syn::Pat::Ident(p) => Ok(p.ident.to_string()),
        syn::Pat::Type(t) => pat_ident(&t.pat),
        other => Err(format!("unsupported let pattern: {other:?}")),
    }
}

fn path_ident(expr: &syn::Expr) -> Result<String, String> {
    match expr {
        syn::Expr::Path(p) => p
            .path
            .get_ident()
            .map(|i| i.to_string())
            .ok_or_else(|| "expected a simple variable".into()),
        other => Err(format!("expected a variable, got {other:?}")),
    }
}

fn path_str(p: &syn::Path) -> Result<String, String> {
    p.get_ident().map(|i| i.to_string()).ok_or_else(|| "expected a struct name".into())
}

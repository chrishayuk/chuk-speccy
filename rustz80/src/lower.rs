//! Lower a `syn` `ItemFn` (accepted subset) to the IR. Unsupported nodes become
//! errors — the "not supported on Z80 / host-only" signal.

use crate::ir::*;
use std::collections::HashMap;

/// Name → (base slot, size in slots). Scalars are size 1; arrays reserve `N`
/// contiguous slots. Flat scoping in Stage 1.
#[derive(Default)]
struct Vars {
    map: HashMap<String, (usize, usize)>,
    next: usize,
}

impl Vars {
    fn declare(&mut self, name: &str, size: usize) -> usize {
        let base = self.next;
        self.map.insert(name.to_string(), (base, size));
        self.next += size;
        base
    }
    /// Base slot of an existing variable (declares a scalar if unseen — rustc
    /// would already have rejected a genuinely-undefined name).
    fn base(&mut self, name: &str) -> usize {
        match self.map.get(name) {
            Some((b, _)) => *b,
            None => self.declare(name, 1),
        }
    }
}

/// Lower every `fn` in a file, in source order, to `(name, Func)`.
pub fn lower_program(file: &syn::File) -> Result<Vec<(String, Func)>, String> {
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(f) => out.push((f.sig.ident.to_string(), lower(f)?)),
            other => return Err(format!("only `fn` items are supported: {other:?}")),
        }
    }
    if out.is_empty() {
        return Err("no functions found".into());
    }
    Ok(out)
}

pub fn lower(item: &syn::ItemFn) -> Result<Func, String> {
    let mut vars = Vars::default();
    let mut body = Vec::new();
    let mut ret = None;

    let mut params = 0;
    for arg in &item.sig.inputs {
        let syn::FnArg::Typed(pt) = arg else {
            return Err("`self` parameters are not supported".into());
        };
        vars.declare(&pat_ident(&pt.pat)?, 1);
        params += 1;
    }
    if params > 3 {
        return Err("more than 3 parameters not supported yet (no stack args)".into());
    }

    let stmts = &item.block.stmts;
    for (i, st) in stmts.iter().enumerate() {
        let last = i + 1 == stmts.len();
        match st {
            syn::Stmt::Local(local) => lower_local(local, &mut vars, &mut body)?,
            syn::Stmt::Expr(expr, semi) => {
                if last && semi.is_none() && is_value_expr(expr) {
                    ret = Some(lower_expr(expr, &mut vars)?);
                } else {
                    lower_stmt_expr(expr, &mut vars, &mut body)?;
                }
            }
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }

    Ok(Func { params, n_locals: vars.next, body, ret })
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
    )
}

/// Lower a `let` — scalar, `[v; N]`, or `[e0, e1, …]`.
fn lower_local(local: &syn::Local, vars: &mut Vars, body: &mut Vec<Stmt>) -> Result<(), String> {
    let init = local.init.as_ref().ok_or("`let` needs an initializer")?;
    let name = pat_ident(&local.pat)?;
    match &*init.expr {
        syn::Expr::Repeat(r) => {
            let n = lit_len(&r.len)?;
            let base = vars.declare(&name, n);
            for i in 0..n {
                let v = lower_expr(&r.expr, vars)?;
                body.push(Stmt::StoreIndex(base, Expr::Lit(i as u16), v));
            }
        }
        syn::Expr::Array(arr) => {
            let base = vars.declare(&name, arr.elems.len());
            for (i, e) in arr.elems.iter().enumerate() {
                let v = lower_expr(e, vars)?;
                body.push(Stmt::StoreIndex(base, Expr::Lit(i as u16), v));
            }
        }
        other => {
            let e = lower_expr(other, vars)?;
            let base = vars.declare(&name, 1);
            body.push(Stmt::Assign(base, e));
        }
    }
    Ok(())
}

fn lower_stmt_expr(expr: &syn::Expr, vars: &mut Vars, body: &mut Vec<Stmt>) -> Result<(), String> {
    match expr {
        syn::Expr::Assign(a) => match &*a.left {
            // `arr[i] = v`
            syn::Expr::Index(ix) => {
                let base = vars.base(&path_ident(&ix.expr)?);
                let idx = lower_expr(&ix.index, vars)?;
                let val = lower_expr(&a.right, vars)?;
                body.push(Stmt::StoreIndex(base, idx, val));
            }
            // `x = v`
            _ => {
                let slot = vars.base(&path_ident(&a.left)?);
                let e = lower_expr(&a.right, vars)?;
                body.push(Stmt::Assign(slot, e));
            }
        },
        syn::Expr::If(ifx) => {
            let cond = lower_cond(&ifx.cond, vars)?;
            let then = lower_block(&ifx.then_branch, vars)?;
            let els = match &ifx.else_branch {
                Some((_, e)) => lower_else(e, vars)?,
                None => Vec::new(),
            };
            body.push(Stmt::If(cond, then, els));
        }
        syn::Expr::While(w) => {
            let cond = lower_cond(&w.cond, vars)?;
            let inner = lower_block(&w.body, vars)?;
            body.push(Stmt::While(cond, inner));
        }
        other => return Err(format!("unsupported statement expression: {other:?}")),
    }
    Ok(())
}

fn lower_else(e: &syn::Expr, vars: &mut Vars) -> Result<Vec<Stmt>, String> {
    match e {
        syn::Expr::Block(b) => lower_block(&b.block, vars),
        syn::Expr::If(_) => {
            let mut v = Vec::new();
            lower_stmt_expr(e, vars, &mut v)?;
            Ok(v)
        }
        other => Err(format!("unsupported else branch: {other:?}")),
    }
}

fn lower_block(b: &syn::Block, vars: &mut Vars) -> Result<Vec<Stmt>, String> {
    let mut body = Vec::new();
    for st in &b.stmts {
        match st {
            syn::Stmt::Local(local) => lower_local(local, vars, &mut body)?,
            syn::Stmt::Expr(expr, _) => lower_stmt_expr(expr, vars, &mut body)?,
            other => return Err(format!("unsupported statement: {other:?}")),
        }
    }
    Ok(body)
}

fn lower_cond(expr: &syn::Expr, vars: &mut Vars) -> Result<Cond, String> {
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
    Ok(Cond { cmp, lhs: lower_expr(&b.left, vars)?, rhs: lower_expr(&b.right, vars)? })
}

fn lower_expr(expr: &syn::Expr, vars: &mut Vars) -> Result<Expr, String> {
    match expr {
        syn::Expr::Lit(l) => {
            let syn::Lit::Int(i) = &l.lit else {
                return Err("only integer literals are supported".into());
            };
            Ok(Expr::Lit(i.base10_parse::<u16>().map_err(|e| e.to_string())?))
        }
        syn::Expr::Path(_) => Ok(Expr::Var(vars.base(&path_ident(expr)?))),
        syn::Expr::Paren(p) => lower_expr(&p.expr, vars),
        // `x as usize` / `as u8` — all values are 16-bit in Stage 1, so a no-op.
        syn::Expr::Cast(c) => lower_expr(&c.expr, vars),
        syn::Expr::Index(ix) => {
            let base = vars.base(&path_ident(&ix.expr)?);
            Ok(Expr::Index(base, Box::new(lower_expr(&ix.index, vars)?)))
        }
        syn::Expr::Binary(b) => {
            let op = match b.op {
                syn::BinOp::Add(_) => BinOp::Add,
                syn::BinOp::Sub(_) => BinOp::Sub,
                syn::BinOp::Mul(_) => BinOp::Mul,
                syn::BinOp::Div(_) => BinOp::Div,
                syn::BinOp::Rem(_) => BinOp::Rem,
                other => return Err(format!("unsupported arithmetic op: {other:?}")),
            };
            Ok(Expr::Bin(op, Box::new(lower_expr(&b.left, vars)?), Box::new(lower_expr(&b.right, vars)?)))
        }
        syn::Expr::Call(c) => {
            let name = path_ident(&c.func)?;
            if c.args.len() > 3 {
                return Err("more than 3 call arguments not supported yet".into());
            }
            let args = c.args.iter().map(|a| lower_expr(a, vars)).collect::<Result<_, _>>()?;
            Ok(Expr::Call(name, args))
        }
        other => Err(format!("unsupported expression: {other:?}")),
    }
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

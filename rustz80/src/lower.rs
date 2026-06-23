//! Lower a `syn` `ItemFn` (accepted subset) to the IR. Unsupported nodes become
//! errors — the "not supported on Z80 / host-only" signal.
//!
//! Structs are a lowering-only concern: every field has a constant offset, so
//! `s.field` lowers to a plain slot access — codegen needs no struct awareness.

use crate::ir::*;
use std::collections::HashMap;

/// One struct field's layout: its name and slot count (1 for a scalar, `N` for a
/// `[u16; N]` array field). Offsets are the running sum of `slots`.
#[derive(Clone)]
struct FieldDef {
    name: String,
    slots: usize,
}

/// Struct layouts: name → fields in declaration order. A field occupies `slots`
/// consecutive `u16` slots; a field's offset is the sum of preceding `slots`.
type Structs = HashMap<String, Vec<FieldDef>>;

/// Total `u16` slots a struct occupies.
fn struct_slots(fields: &[FieldDef]) -> usize {
    fields.iter().map(|f| f.slots).sum()
}

/// C-like enum layouts: name → variants as `(name, value)`. Values follow Rust's
/// rule: an explicit `= N` discriminant, else the previous value + 1 (from 0).
type Enums = HashMap<String, Vec<(String, u16)>>;

/// Caller-supplied routing for "handle" parameters — method receivers whose type is
/// a handle (e.g. an SDK `Frame`/`Input`) route a method call to a free prelude
/// function, *dropping* the receiver (the 3-register calling convention has no room
/// for `self` + 3 args). This keeps the compiler generic: it knows nothing about any
/// particular SDK; the SDK (or any caller) provides the map. Generic compilation
/// passes an empty config — then there are no handle types.
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
    fn lookup(&self, handle: &str, method: &str) -> Option<&str> {
        self.routes
            .get(&(handle.to_string(), method.to_string()))
            .map(String::as_str)
    }
}

/// Per-function lowering context: locals + the program's struct/enum layouts + the
/// caller's handle-routing config.
struct Ctx<'a> {
    vars: Vars,
    structs: &'a Structs,
    enums: &'a Enums,
    prelude: &'a PreludeConfig,
    /// Counter for synthesised `match` scrutinee temporaries.
    temp: usize,
}

struct VarInfo {
    base: usize,
    sty: Option<String>,
    ty: Width,    // scalar value type, or array element type
    is_ptr: bool, // a pointer to a struct (e.g. `self`) vs a by-value struct local
    /// A prelude handle type (`"Frame"`/`"Input"`) — methods route to intrinsics.
    handle: Option<String>,
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
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty,
                ty,
                is_ptr: false,
                handle: None,
            },
        );
        self.next += size;
        base
    }
    /// Declare a pointer-to-struct local (one slot holding an address) — `self`.
    fn declare_ptr(&mut self, name: &str, sty: &str) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty: Some(sty.to_string()),
                ty: Width::Word,
                is_ptr: true,
                handle: None,
            },
        );
        self.next += 1;
        base
    }
    /// Declare a prelude-handle param (`frame: &mut Frame`, `input: &Input`).
    fn declare_handle(&mut self, name: &str, handle: &str) -> usize {
        let base = self.next;
        self.map.insert(
            name.to_string(),
            VarInfo {
                base,
                sty: None,
                ty: Width::Word,
                is_ptr: false,
                handle: Some(handle.to_string()),
            },
        );
        self.next += 1;
        base
    }
    fn handle_of(&self, name: &str) -> Option<String> {
        self.map.get(name).and_then(|v| v.handle.clone())
    }
    fn base(&mut self, name: &str) -> usize {
        match self.map.get(name) {
            Some(v) => v.base,
            None => self.declare(name, 1, None, Width::Word),
        }
    }
    /// A struct-typed var as a method receiver: `(base, struct name, is_ptr)`.
    fn receiver(&self, name: &str) -> Option<(usize, String, bool)> {
        self.map
            .get(name)
            .and_then(|v| v.sty.as_ref().map(|s| (v.base, s.clone(), v.is_ptr)))
    }
    /// The variable's value type (scalar) or element type (array).
    fn ty(&self, name: &str) -> Width {
        self.map.get(name).map(|v| v.ty).unwrap_or(Width::Word)
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
    let mut out = Vec::new();
    for item in &file.items {
        match item {
            // `poke`/`peek` are host-only prelude intrinsics — skip their bodies.
            syn::Item::Fn(f) if is_intrinsic(&f.sig.ident.to_string()) => {}
            syn::Item::Fn(f) => out.push((
                f.sig.ident.to_string(),
                lower_with(f, &structs, &enums, prelude)?,
            )),
            // `impl T { fn m(&mut self, …) }` — each method becomes a `T::m` function
            // taking `self` as a leading pointer argument.
            syn::Item::Impl(imp) => {
                let self_ty = type_name(&imp.self_ty)?;
                for it in &imp.items {
                    let syn::ImplItem::Fn(m) = it else {
                        return Err("only methods are supported in impl blocks".into());
                    };
                    let name = format!("{self_ty}::{}", m.sig.ident);
                    out.push((name, lower_method(m, &self_ty, &structs, &enums, prelude)?));
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
    if out.is_empty() {
        return Err("no functions found".into());
    }
    Ok(out)
}

/// Lower a standalone function (no struct/enum context — used by `compile_fn`).
pub fn lower(item: &syn::ItemFn) -> Result<Func, String> {
    lower_with(
        item,
        &Structs::new(),
        &Enums::new(),
        &PreludeConfig::default(),
    )
}

/// Lower an `impl` method. The receiver (`&self`/`&mut self`) becomes a leading
/// pointer parameter; `self.field` reads/writes through it.
fn lower_method(
    m: &syn::ImplItemFn,
    self_ty: &str,
    structs: &Structs,
    enums: &Enums,
    prelude: &PreludeConfig,
) -> Result<Func, String> {
    let mut ctx = Ctx {
        vars: Vars::default(),
        structs,
        enums,
        prelude,
        temp: 0,
    };
    let params = lower_inputs(&m.sig.inputs, &mut ctx, Some(self_ty))?;
    let (body, ret) = lower_fn_block(&m.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
}

/// Names the compiler handles itself (their host definitions are prelude-only).
fn is_intrinsic(name: &str) -> bool {
    matches!(name, "poke" | "peek" | "inport")
}

fn collect_enums(file: &syn::File) -> Result<Enums, String> {
    let mut m = Enums::new();
    for item in &file.items {
        if let syn::Item::Enum(e) = item {
            let mut variants = Vec::new();
            let mut next = 0u16;
            for v in &e.variants {
                let value = match &v.discriminant {
                    Some((_, expr)) => lit_u16(expr)?,
                    None => next,
                };
                variants.push((v.ident.to_string(), value));
                next = value.wrapping_add(1);
            }
            m.insert(e.ident.to_string(), variants);
        }
    }
    Ok(m)
}

fn lit_u16(e: &syn::Expr) -> Result<u16, String> {
    if let syn::Expr::Lit(l) = e {
        if let syn::Lit::Int(i) = &l.lit {
            return i.base10_parse::<u16>().map_err(|e| e.to_string());
        }
    }
    Err("enum discriminant must be an integer literal".into())
}

fn collect_structs(file: &syn::File) -> Result<Structs, String> {
    let mut m = Structs::new();
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            let syn::Fields::Named(named) = &s.fields else {
                return Err(format!(
                    "only named-field structs are supported: {}",
                    s.ident
                ));
            };
            let mut fields = Vec::new();
            for f in &named.named {
                // Scalar fields are one slot; `[u16; N]` array fields are N slots.
                // Other shapes would mislay offsets, so reject them clearly.
                let slots = match &f.ty {
                    syn::Type::Path(_) => 1,
                    syn::Type::Array(arr) if is_u16(&arr.elem) => lit_len(&arr.len)?,
                    syn::Type::Array(_) => {
                        return Err(format!(
                            "only `[u16; N]` array struct fields are supported: {}",
                            s.ident
                        ))
                    }
                    _ => {
                        return Err(format!(
                            "only scalar or `[u16; N]` struct fields are supported: {}",
                            s.ident
                        ))
                    }
                };
                fields.push(FieldDef {
                    name: f.ident.as_ref().unwrap().to_string(),
                    slots,
                });
            }
            m.insert(s.ident.to_string(), fields);
        }
    }
    Ok(m)
}

fn lower_with(
    item: &syn::ItemFn,
    structs: &Structs,
    enums: &Enums,
    prelude: &PreludeConfig,
) -> Result<Func, String> {
    let mut ctx = Ctx {
        vars: Vars::default(),
        structs,
        enums,
        prelude,
        temp: 0,
    };
    let params = lower_inputs(&item.sig.inputs, &mut ctx, None)?;
    let (body, ret) = lower_fn_block(&item.block, &mut ctx)?;
    Ok(Func {
        params,
        n_locals: ctx.vars.next,
        body,
        ret,
    })
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
                    None => ctx.vars.declare(&name, 1, None, ty_of_type(&pt.ty)),
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
            let fields = ctx
                .structs
                .get(&sname)
                .ok_or_else(|| format!("unknown struct {sname}"))?
                .clone();
            let base = ctx.vars.declare(
                &name,
                struct_slots(&fields),
                Some(sname.clone()),
                Width::Word,
            );
            for fv in &lit.fields {
                let fname = member_name(&fv.member)?;
                if fields
                    .iter()
                    .find(|f| f.name == fname)
                    .is_some_and(|f| f.slots != 1)
                {
                    return Err(format!(
                        "init array field `{fname}` by index, not a struct literal"
                    ));
                }
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
                body.push(lower_index_store(ix, &a.right, ctx)?);
            }
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
            let inner = lower_block(&w.body, ctx)?;
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

/// Lower an expression, returning its IR and inferred width (`u8`/`u16`).
fn lower_expr(expr: &syn::Expr, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
    match expr {
        syn::Expr::Lit(l) => match &l.lit {
            syn::Lit::Int(i) => {
                let w = if i.suffix() == "u8" {
                    Width::Byte
                } else {
                    Width::Word
                };
                Ok((
                    Expr::Lit(i.base10_parse::<u16>().map_err(|e| e.to_string())?),
                    w,
                ))
            }
            syn::Lit::Bool(b) => Ok((Expr::Lit(b.value as u16), Width::Byte)),
            other => Err(format!("unsupported literal: {other:?}")),
        },
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
        syn::Expr::Field(f) => Ok((lower_field_read(f, ctx)?, Width::Word)),
        syn::Expr::Index(ix) => lower_index_read(ix, ctx),
        syn::Expr::Binary(b) => {
            let op = bin_op(&b.op)?;
            let (le, lw) = lower_expr(&b.left, ctx)?;
            let (re, _) = lower_expr(&b.right, ctx)?;
            Ok((Expr::Bin(op, Box::new(le), Box::new(re), lw), lw))
        }
        syn::Expr::MethodCall(m) => lower_method_call(m, ctx),
        syn::Expr::Call(c) => {
            let name = path_ident(&c.func)?;
            // `peek(addr)` intrinsic — read a byte from raw memory.
            if name == "peek" {
                let addr = c.args.first().ok_or("peek(addr) needs an address")?;
                return Ok((Expr::Peek(Box::new(lower_expr(addr, ctx)?.0)), Width::Byte));
            }
            if name == "inport" {
                let port = c.args.first().ok_or("inport(port) needs a port")?;
                return Ok((
                    Expr::InPort(Box::new(lower_expr(port, ctx)?.0)),
                    Width::Byte,
                ));
            }
            if c.args.len() > 3 {
                return Err("more than 3 call arguments not supported yet".into());
            }
            let args = c
                .args
                .iter()
                .map(|a| Ok(lower_expr(a, ctx)?.0))
                .collect::<Result<_, String>>()?;
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
        syn::BinOp::BitOr(_) => BinOp::Or,
        syn::BinOp::BitAnd(_) => BinOp::And,
        syn::BinOp::BitXor(_) => BinOp::Xor,
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

/// Resolve a field access `obj.field` to `(obj base slot, field offset, is_ptr)`.
fn field_target(f: &syn::ExprField, ctx: &mut Ctx) -> Result<(usize, usize, bool), String> {
    let obj = path_ident(&f.base)?;
    let (base, sname, is_ptr) = ctx
        .vars
        .receiver(&obj)
        .ok_or_else(|| format!("{obj} is not a struct"))?;
    let fields = ctx
        .structs
        .get(&sname)
        .ok_or_else(|| format!("unknown struct {sname}"))?;
    let off = field_offset(fields, &member_name(&f.member)?)?;
    Ok((base, off, is_ptr))
}

/// Lower an index read `base[idx]`: a local array (`arr[i]`) or an array *field*
/// reached through a struct receiver (`self.arr[i]`).
fn lower_index_read(ix: &syn::ExprIndex, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
    if let syn::Expr::Field(f) = &*ix.expr {
        let (base, off, is_ptr) = field_target(f, ctx)?;
        let idx = lower_expr(&ix.index, ctx)?.0;
        let e = if is_ptr {
            // `self.arr[i]` → *(self + off*2 + i*2)
            Expr::PtrIndex {
                ptr: Box::new(Expr::Var(base)),
                off: off * 2,
                index: Box::new(idx),
            }
        } else {
            // by-value struct local: the array's first slot is `base + off`.
            Expr::Index(base + off, Box::new(idx), Width::Word)
        };
        return Ok((e, Width::Word));
    }
    let arr = path_ident(&ix.expr)?;
    let base = ctx.vars.base(&arr);
    let w = ctx.vars.ty(&arr);
    let idx = lower_expr(&ix.index, ctx)?.0;
    Ok((Expr::Index(base, Box::new(idx), w), w))
}

/// Lower an index store `base[idx] = rhs` (mirror of [`lower_index_read`]).
fn lower_index_store(ix: &syn::ExprIndex, rhs: &syn::Expr, ctx: &mut Ctx) -> Result<Stmt, String> {
    if let syn::Expr::Field(f) = &*ix.expr {
        let (base, off, is_ptr) = field_target(f, ctx)?;
        let idx = lower_expr(&ix.index, ctx)?.0;
        let val = lower_expr(rhs, ctx)?.0;
        return Ok(if is_ptr {
            Stmt::PtrStoreIndex {
                ptr: Box::new(Expr::Var(base)),
                off: off * 2,
                index: Box::new(idx),
                value: val,
            }
        } else {
            Stmt::StoreIndex(base + off, idx, val, Width::Word)
        });
    }
    let arr = path_ident(&ix.expr)?;
    let base = ctx.vars.base(&arr);
    let w = ctx.vars.ty(&arr);
    let idx = lower_expr(&ix.index, ctx)?.0;
    let val = lower_expr(rhs, ctx)?.0;
    Ok(Stmt::StoreIndex(base, idx, val, w))
}

/// Read `obj.field` — a constant slot for a by-value struct, an indirect load
/// through the pointer for `self`-style receivers.
fn lower_field_read(f: &syn::ExprField, ctx: &mut Ctx) -> Result<Expr, String> {
    let (base, off, is_ptr) = field_target(f, ctx)?;
    if is_ptr {
        Ok(Expr::Deref(Box::new(Expr::Var(base)), off * 2))
    } else {
        Ok(Expr::Var(base + off))
    }
}

/// Write `obj.field = val`.
fn lower_field_store(f: &syn::ExprField, val: Expr, ctx: &mut Ctx) -> Result<Stmt, String> {
    let (base, off, is_ptr) = field_target(f, ctx)?;
    if is_ptr {
        Ok(Stmt::Store(Expr::Var(base), off * 2, val))
    } else {
        Ok(Stmt::Assign(base + off, val))
    }
}

/// Lower a method call: the `wrapping_*` value ops, or `obj.m(a, b)` →
/// `Type::m(&obj, a, b)` (`self` passed as a leading pointer).
fn lower_method_call(m: &syn::ExprMethodCall, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
    let method = m.method.to_string();
    if let "wrapping_add" | "wrapping_sub" | "wrapping_mul" = method.as_str() {
        let op = match method.as_str() {
            "wrapping_add" => BinOp::Add,
            "wrapping_sub" => BinOp::Sub,
            _ => BinOp::Mul,
        };
        let (recv, rw) = lower_expr(&m.receiver, ctx)?;
        let arg = m.args.first().ok_or("wrapping_* needs an argument")?;
        let (re, _) = lower_expr(arg, ctx)?;
        return Ok((Expr::Bin(op, Box::new(recv), Box::new(re), rw), rw));
    }
    let recv = path_ident(&m.receiver)?;
    // Prelude handles (`frame`/`input`): route methods to intrinsic prelude fns.
    if let Some(handle) = ctx.vars.handle_of(&recv) {
        return lower_prelude_call(&handle, &method, &m.args, ctx);
    }
    let (base, sname, is_ptr) = ctx
        .vars
        .receiver(&recv)
        .ok_or_else(|| format!("method receiver {recv} is not a struct"))?;
    let self_ptr = if is_ptr {
        Expr::Var(base)
    } else {
        Expr::AddrOf(base)
    };
    let mut args = vec![self_ptr];
    for a in &m.args {
        args.push(lower_expr(a, ctx)?.0);
    }
    if args.len() > 3 {
        return Err("method receiver + args exceed 3 registers".into());
    }
    Ok((Expr::Call(format!("{sname}::{method}"), args), Width::Word))
}

/// A handle parameter type (`&mut T`/`&T`/`T` → `"T"`) if `T` is a configured
/// handle type (e.g. the SDK's `Frame`/`Input`); otherwise `None`.
fn handle_type(t: &syn::Type, prelude: &PreludeConfig) -> Option<String> {
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

/// Route `<handle>.<method>(args)` to the configured prelude function (the receiver
/// is dropped — see [`PreludeConfig`]).
fn lower_prelude_call(
    handle: &str,
    method: &str,
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
    ctx: &mut Ctx,
) -> Result<(Expr, Width), String> {
    let name = ctx
        .prelude
        .lookup(handle, method)
        .ok_or_else(|| format!("prelude method {handle}::{method} is not routed"))?
        .to_string();
    let lowered = args
        .iter()
        .map(|a| Ok(lower_expr(a, ctx)?.0))
        .collect::<Result<_, String>>()?;
    Ok((Expr::Call(name, lowered), Width::Word))
}

/// The simple name of an `impl` target type (`impl Foo` → `Foo`).
fn type_name(t: &syn::Type) -> Result<String, String> {
    if let syn::Type::Path(p) = t {
        if let Some(id) = p.path.get_ident() {
            return Ok(id.to_string());
        }
    }
    Err(format!("unsupported impl type: {t:?}"))
}

/// `Enum::Variant` (a 2-segment path) → its integer value, if known.
fn resolve_enum_path(path: &syn::Path, enums: &Enums) -> Option<u16> {
    if path.segments.len() != 2 {
        return None;
    }
    let name = path.segments[0].ident.to_string();
    let variant = path.segments[1].ident.to_string();
    enums
        .get(&name)?
        .iter()
        .find(|(n, _)| *n == variant)
        .map(|(_, v)| *v)
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

/// A field's slot offset: the running sum of preceding fields' slots (so array
/// fields shift everything after them by their length).
fn field_offset(fields: &[FieldDef], name: &str) -> Result<usize, String> {
    let mut off = 0;
    for f in fields {
        if f.name == name {
            return Ok(off);
        }
        off += f.slots;
    }
    Err(format!("no field {name}"))
}

/// Is `ty` the `u16` path type?
fn is_u16(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(p) if p.path.is_ident("u16"))
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
    p.get_ident()
        .map(|i| i.to_string())
        .ok_or_else(|| "expected a struct name".into())
}

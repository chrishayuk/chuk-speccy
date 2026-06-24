//! Expression lowering: a `syn::Expr` → IR [`Expr`] plus its inferred [`Width`].
//! Also the field/index access helpers (constant slot for a by-value struct, an
//! indirect load/store through the pointer for `self`-style receivers) and method
//! calls (`wrapping_*`, prelude-handle routing, or `obj.m(a) → Type::m(&obj, a)`).

use super::generics::{call_target, resolve_generic};
use super::layout::{field_offset, member_name, resolve_enum_path, struct_slots};
use super::Ctx;
use crate::ir::*;

/// The byte address of `a[i].field` for a local struct-element array `[Cell; N]`:
/// `&a + index*(elem_stride) + field_offset` (all in bytes). Errs if `a` isn't a
/// struct-element array.
pub(crate) fn elem_field_addr(
    ix: &syn::ExprIndex,
    member: &syn::Member,
    ctx: &mut Ctx,
) -> Result<Expr, String> {
    let arr = path_ident(&ix.expr)?;
    let elem_struct = ctx
        .vars
        .elem_struct(&arr)
        .ok_or_else(|| format!("`{arr}` is not a struct-element array"))?;
    let base = ctx.vars.base(&arr);
    let efields = ctx
        .struct_fields(&elem_struct)
        .ok_or_else(|| format!("unknown struct {elem_struct}"))?;
    let foff = field_offset(&efields, &member_name(member)?)?;
    let stride = (struct_slots(&efields) * 2) as u16;
    let idx = lower_expr(&ix.index, ctx)?.0;
    // &a + index*stride
    let elem = Expr::Bin(
        BinOp::Add,
        Box::new(Expr::AddrOf(base)),
        Box::new(Expr::MulConst(Box::new(idx), stride)),
        Width::Word,
    );
    Ok(if foff == 0 {
        elem
    } else {
        Expr::Bin(
            BinOp::Add,
            Box::new(elem),
            Box::new(Expr::Lit((foff * 2) as u16)),
            Width::Word,
        )
    })
}

/// Lower an expression, returning its IR and inferred width (`u8`/`u16`).
pub(crate) fn lower_expr(expr: &syn::Expr, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
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
                // A const-generic parameter is substituted by its instance value.
                if let Some(v) = ctx.const_args.get(&name) {
                    return Ok((Expr::Lit(*v), Width::Word));
                }
                Ok((Expr::Var(ctx.vars.base(&name)), ctx.vars.ty(&name)))
            }
        },
        syn::Expr::Paren(p) => lower_expr(&p.expr, ctx),
        // `e as u8` truncates to a byte; `as u16`/`as usize` is a no-op (16-bit).
        syn::Expr::Cast(c) => {
            let (e, _) = lower_expr(&c.expr, ctx)?;
            if ctx.width_of_type(&c.ty) == Width::Byte {
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
            let (name, turbofish) = call_target(&c.func)?;
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
            let lowered = c
                .args
                .iter()
                .map(|a| lower_expr(a, ctx))
                .collect::<Result<Vec<_>, String>>()?;
            let args: Vec<Expr> = lowered.iter().map(|(e, _)| e.clone()).collect();

            // A call to a generic function instantiates a specialized copy.
            let is_generic = ctx.mono.borrow().generics.contains_key(&name);
            if is_generic {
                let (gargs, ret_w) = resolve_generic(&name, &turbofish, &lowered, ctx)?;
                let inst = ctx.mono.borrow_mut().request(&name, gargs);
                return Ok((Expr::Call(inst, args), ret_w));
            }
            if !turbofish.is_empty() {
                return Err(format!("`{name}` is not a generic function"));
            }
            Ok((Expr::Call(name, args), Width::Word)) // non-generic calls assume u16 returns
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

/// Resolve a field access to `(obj base slot, slot offset, is_ptr, slot count)`.
/// Handles `obj.field` and a tuple element of a struct field, `obj.field.N`.
fn field_target(f: &syn::ExprField, ctx: &mut Ctx) -> Result<(usize, usize, bool, usize), String> {
    // `obj.field.N` — a tuple element (one slot) at the field's offset + N.
    if let syn::Expr::Field(inner) = &*f.base {
        let syn::Member::Unnamed(idx) = &f.member else {
            return Err("nested struct fields are not supported".into());
        };
        let (base, off, is_ptr, _) = field_target(inner, ctx)?;
        return Ok((base, off + idx.index as usize, is_ptr, 1));
    }
    let obj = path_ident(&f.base)?;
    let (base, sname, is_ptr) = ctx
        .vars
        .receiver(&obj)
        .ok_or_else(|| format!("{obj} is not a struct"))?;
    let fields = ctx
        .struct_fields(&sname)
        .ok_or_else(|| format!("unknown struct {sname}"))?;
    let name = member_name(&f.member)?;
    let off = field_offset(&fields, &name)?;
    let slots = fields
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.slots)
        .unwrap_or(1);
    Ok((base, off, is_ptr, slots))
}

/// Lower an index read `base[idx]`: a local array (`arr[i]`) or an array *field*
/// reached through a struct receiver (`self.arr[i]`).
fn lower_index_read(ix: &syn::ExprIndex, ctx: &mut Ctx) -> Result<(Expr, Width), String> {
    if let syn::Expr::Field(f) = &*ix.expr {
        let (base, off, is_ptr, _) = field_target(f, ctx)?;
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
    if ctx.vars.elem_struct(&arr).is_some() {
        return Err(format!(
            "a struct-array element isn't a scalar — read a field, e.g. `{arr}[i].x`"
        ));
    }
    let base = ctx.vars.base(&arr);
    let w = ctx.vars.ty(&arr);
    let idx = lower_expr(&ix.index, ctx)?.0;
    Ok((Expr::Index(base, Box::new(idx), w), w))
}

/// Lower an index store `base[idx] = rhs` (mirror of [`lower_index_read`]).
pub(crate) fn lower_index_store(
    ix: &syn::ExprIndex,
    rhs: &syn::Expr,
    ctx: &mut Ctx,
) -> Result<Stmt, String> {
    if let syn::Expr::Field(f) = &*ix.expr {
        let (base, off, is_ptr, _) = field_target(f, ctx)?;
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
    // `a[i].field` — a field of a struct-array element at a computed address.
    if let syn::Expr::Index(ix) = &*f.base {
        return Ok(Expr::LoadAt(
            Box::new(elem_field_addr(ix, &f.member, ctx)?),
            Width::Word,
        ));
    }
    let (base, off, is_ptr, slots) = field_target(f, ctx)?;
    if slots != 1 {
        return Err("this field is not a scalar (read a tuple field by element: `.0`)".into());
    }
    if is_ptr {
        Ok(Expr::Deref(Box::new(Expr::Var(base)), off * 2))
    } else {
        Ok(Expr::Var(base + off))
    }
}

/// Write `obj.field = val`.
pub(crate) fn lower_field_store(
    f: &syn::ExprField,
    val: Expr,
    ctx: &mut Ctx,
) -> Result<Stmt, String> {
    // `a[i].field = v` — store a field of a struct-array element at a computed address.
    if let syn::Expr::Index(ix) = &*f.base {
        return Ok(Stmt::StoreAt(
            elem_field_addr(ix, &f.member, ctx)?,
            val,
            Width::Word,
        ));
    }
    let (base, off, is_ptr, slots) = field_target(f, ctx)?;
    if slots != 1 {
        return Err("this field is not a scalar (assign a tuple field by element: `.0`)".into());
    }
    if is_ptr {
        Ok(Stmt::Store(Expr::Var(base), off * 2, val))
    } else {
        Ok(Stmt::Assign(base + off, val))
    }
}

/// Lower a method call: the `wrapping_*` value ops, or `obj.m(a, b)` →
/// `Type::m(&obj, a, b)` (`self` passed as a leading pointer).
pub(crate) fn lower_method_call(
    m: &syn::ExprMethodCall,
    ctx: &mut Ctx,
) -> Result<(Expr, Width), String> {
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

/// Route `<handle>.<method>(args)` to the configured prelude function (the receiver
/// is dropped — see [`super::PreludeConfig`]).
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

pub(crate) fn path_ident(expr: &syn::Expr) -> Result<String, String> {
    match expr {
        syn::Expr::Path(p) => p
            .path
            .get_ident()
            .map(|i| i.to_string())
            .ok_or_else(|| "expected a simple variable".into()),
        other => Err(format!("expected a variable, got {other:?}")),
    }
}

pub(crate) fn path_str(p: &syn::Path) -> Result<String, String> {
    p.get_ident()
        .map(|i| i.to_string())
        .ok_or_else(|| "expected a struct name".into())
}

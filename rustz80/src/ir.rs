//! The compiler's own small typed IR — decoupled from `syn`. Stage 0 is `u16`
//! throughout (8-bit narrowing comes later); locals are addressed by slot.

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
}

#[derive(Debug, Clone, Copy)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone)]
pub enum Expr {
    /// An integer literal.
    Lit(u16),
    /// A local variable, by slot index.
    Var(usize),
    /// A binary arithmetic op.
    Bin(BinOp, Box<Expr>, Box<Expr>),
}

/// A boolean condition (a single comparison — no `&&`/`||` in Stage 0).
#[derive(Debug, Clone)]
pub struct Cond {
    pub cmp: Cmp,
    pub lhs: Expr,
    pub rhs: Expr,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// Store an expression into a local slot (covers `let` and reassignment).
    Assign(usize, Expr),
    /// `if cond { then } else { els }`.
    If(Cond, Vec<Stmt>, Vec<Stmt>),
    /// `while cond { body }`.
    While(Cond, Vec<Stmt>),
}

/// A lowered function: its locals, body, and optional tail-expression result.
#[derive(Debug, Clone)]
pub struct Func {
    pub n_locals: usize,
    pub body: Vec<Stmt>,
    pub ret: Option<Expr>,
}

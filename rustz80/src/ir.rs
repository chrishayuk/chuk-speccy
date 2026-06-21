//! The compiler's own small typed IR — decoupled from `syn`. Stage 0 is `u16`
//! throughout (8-bit narrowing comes later); locals are addressed by slot.

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// Element width for array access. Both kinds occupy a 2-byte slot per element
/// (1 element per slot); only the load/store size differs (`u8` zero-extends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
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
    /// A binary arithmetic op; `Width::Byte` masks the result to 8 bits (u8 wrap).
    Bin(BinOp, Box<Expr>, Box<Expr>, Width),
    /// A call to another function by name (args by the calling convention).
    Call(String, Vec<Expr>),
    /// Read array element `base[index]` (`base` is the array's first slot).
    Index(usize, Box<Expr>, Width),
    /// Truncate to 8 bits (`expr as u8`).
    Trunc(Box<Expr>),
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
    /// Store into array element `base[index] = value`.
    StoreIndex(usize, Expr, Expr, Width),
    /// `if cond { then } else { els }`.
    If(Cond, Vec<Stmt>, Vec<Stmt>),
    /// `while cond { body }`.
    While(Cond, Vec<Stmt>),
}

/// A lowered function. Parameters occupy local slots `0..params` (loaded from
/// the calling-convention registers in the prologue).
#[derive(Debug, Clone)]
pub struct Func {
    pub params: usize,
    pub n_locals: usize,
    pub body: Vec<Stmt>,
    pub ret: Option<Expr>,
}

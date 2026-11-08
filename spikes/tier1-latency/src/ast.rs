//! The syntax tree of the part of M1 the spike reads.

/// A byte range of the module source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// An inclusive range of integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub lo: i128,
    pub hi: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// An integer type: `base` is its plain range, `refined` the range a refinement names.
    Int {
        name: String,
        base: Range,
        refined: Option<Range>,
    },
    Bool,
    Array {
        len: i128,
        elem: Box<Ty>,
    },
    Unit,
}

impl Ty {
    /// The range a value must lie in to enter this type, and whether Tier 1 must prove it
    /// (a refined type) or may leave it to a run-time check (a plain type).
    pub fn int_range(&self) -> Option<(Range, bool)> {
        match self {
            Ty::Int { base, refined, .. } => Some((refined.unwrap_or(*base), refined.is_some())),
            _ => None,
        }
    }

    /// The type without its refinement.
    pub fn plain(&self) -> Ty {
        match self {
            Ty::Int { name, base, .. } => Ty::Int {
                name: name.clone(),
                base: *base,
                refined: None,
            },
            other => other.clone(),
        }
    }
}

/// The plain range of an integer type name, or None.
pub fn int_base(name: &str) -> Option<Range> {
    let unsigned = |bits: u32| Range {
        lo: 0,
        hi: (1i128 << bits) - 1,
    };
    let signed = |bits: u32| Range {
        lo: -(1i128 << (bits - 1)),
        hi: (1i128 << (bits - 1)) - 1,
    };
    Some(match name {
        "u8" => unsigned(8),
        "u16" => unsigned(16),
        "u32" => unsigned(32),
        "u64" | "usize" => unsigned(64),
        "i8" => signed(8),
        "i16" => signed(16),
        "i32" => signed(32),
        "i64" => signed(64),
        // i128 and u128 are left out: the octagon's bounds are i128 and must hold twice a value.
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    BitNot,
    Not,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Int(i128),
    Bool(bool),
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Len(Box<Expr>),
    Call(String, Vec<Expr>),
    ArrayRepeat(Box<Expr>, i128),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub indices: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    Let {
        mutable: bool,
        name: String,
        ty: Option<Ty>,
        init: Expr,
    },
    Assign {
        target: Place,
        op: Option<BinOp>,
        value: Expr,
    },
    While {
        cond: Expr,
        invariants: Vec<Expr>,
        decreases: Option<Expr>,
        body: Vec<Stmt>,
    },
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
    Guard {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Ty,
}

/// What callers see of a function: its name, parameters, result and contracts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Ty,
    pub requires: Vec<Expr>,
    pub ensures: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub sig: Signature,
    pub body: Vec<Stmt>,
    /// The whole function, from `fn` (or its doc comment) to its closing brace.
    pub span: Span,
    /// A digest of the header's tokens, which changes when the signature or a contract does.
    pub header_hash: u64,
    /// A digest of the body's tokens.
    pub body_hash: u64,
    /// Where the body's opening brace starts.
    pub body_start: usize,
}

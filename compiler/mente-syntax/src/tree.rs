//! The syntax tree of one M1 function (`spec/grammar.md`, items 4 to 7).
//!
//! The nodes of a function live in the tables of its [`Tree`] and refer to each other by index,
//! as `MirRegionSoA` does, so the tree has no pointers and no lifetimes. A node keeps the span
//! of the text it was parsed from. [`Tree::render_expr`] prints an expression in one canonical
//! form, fully parenthesised, which the tests compare against.

use crate::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StmtId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

/// The node tables of one function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub exprs: Vec<Expr>,
    pub stmts: Vec<Stmt>,
    pub blocks: Vec<Block>,
    pub types: Vec<TypeExpr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-x`
    Neg,
    /// `~x`
    BitNot,
    /// `not x`
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinaryOp {
    /// The operator as it is written.
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Or => "or",
            BinaryOp::And => "and",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::BitAnd => "&",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
        }
    }
}

/// `=` or a compound assignment; the compound ones carry their operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Compound(BinaryOp),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arg {
    /// `f(mut x)`: the argument is a place the call may change.
    pub is_mut: bool,
    pub value: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// An integer literal: its value, and its suffix when it has one (`u8` for `255_u8`).
    Int {
        value: u128,
        suffix: Option<String>,
    },
    Bool(bool),
    Name(String),
    /// `todo("reason")`.
    Todo(String),
    /// `unreachable("reason")`.
    Unreachable(String),
    Unary {
        op: UnaryOp,
        operand: ExprId,
    },
    Binary {
        op: BinaryOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    /// `a.len`, the only field M1 has.
    Field {
        base: ExprId,
        name: String,
    },
    Call {
        callee: String,
        args: Vec<Arg>,
    },
    Array(Vec<ExprId>),
    /// `[value; count]`, with `count` an integer literal.
    Repeat {
        value: ExprId,
        count: u128,
    },
    If {
        cond: ExprId,
        then: BlockId,
        otherwise: Option<Else>,
    },
    /// What a diagnostic replaced; nothing is checked inside it.
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Else {
    Block(BlockId),
    /// `else if ...`
    If(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StmtKind {
    /// `val name = init` or `var name: ty = init`.
    Binding {
        is_var: bool,
        name: String,
        name_span: Span,
        ty: Option<TypeId>,
        init: ExprId,
    },
    Assign {
        place: ExprId,
        op: AssignOp,
        value: ExprId,
    },
    While {
        cond: ExprId,
        invariants: Vec<ExprId>,
        decreases: Vec<ExprId>,
        body: BlockId,
    },
    Guard {
        cond: ExprId,
        otherwise: BlockId,
    },
    Return(Option<ExprId>),
    Break,
    Continue,
    /// `_ = e`: a value discarded on purpose.
    Discard(ExprId),
    Expr(ExprId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub stmts: Vec<StmtId>,
    pub span: Span,
}

/// A refinement bound: an integer literal, a name, or `name.len`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    Int { negative: bool, value: u128 },
    Name(String),
    Len(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refinement {
    pub lo: Bound,
    pub hi: Bound,
    /// `..=` rather than `..`.
    pub inclusive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    /// An integer type by name (`u8`, `usize`), with its refinement.
    Int {
        name: String,
        refinement: Option<Refinement>,
    },
    Bool,
    Array {
        len: u128,
        elem: TypeId,
    },
    /// What a diagnostic replaced.
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub is_mut: bool,
    pub name: String,
    pub name_span: Span,
    pub ty: TypeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractKind {
    Requires,
    Ensures,
    Decreases,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub kind: ContractKind,
    pub expr: ExprId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Block(BlockId),
    /// `= expression`
    Expr(ExprId),
}

/// One function, parsed under M1's grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnSyntax {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub result: Option<TypeId>,
    pub contracts: Vec<Contract>,
    pub body: Option<Body>,
    pub tree: Tree,
}

impl Tree {
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id.0 as usize]
    }

    pub fn stmt(&self, id: StmtId) -> &Stmt {
        &self.stmts[id.0 as usize]
    }

    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.0 as usize]
    }

    pub fn ty(&self, id: TypeId) -> &TypeExpr {
        &self.types[id.0 as usize]
    }

    pub fn push_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.exprs.push(Expr { kind, span });
        ExprId(self.exprs.len() as u32 - 1)
    }

    pub fn push_stmt(&mut self, kind: StmtKind, span: Span) -> StmtId {
        self.stmts.push(Stmt { kind, span });
        StmtId(self.stmts.len() as u32 - 1)
    }

    pub fn push_block(&mut self, block: Block) -> BlockId {
        self.blocks.push(block);
        BlockId(self.blocks.len() as u32 - 1)
    }

    pub fn push_type(&mut self, kind: TypeKind, span: Span) -> TypeId {
        self.types.push(TypeExpr { kind, span });
        TypeId(self.types.len() as u32 - 1)
    }

    /// An expression in canonical form: every operator application in parentheses, as
    /// `(+ a (* b c))`, so precedence and grouping can be read off.
    pub fn render_expr(&self, id: ExprId) -> String {
        match &self.expr(id).kind {
            ExprKind::Int { value, suffix } => match suffix {
                Some(s) => format!("{}_{}", value, s),
                None => value.to_string(),
            },
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Name(n) => n.clone(),
            ExprKind::Todo(reason) => format!("(todo {:?})", reason),
            ExprKind::Unreachable(reason) => format!("(unreachable {:?})", reason),
            ExprKind::Unary { op, operand } => {
                let sym = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::BitNot => "~",
                    UnaryOp::Not => "not",
                };
                format!("({} {})", sym, self.render_expr(*operand))
            }
            ExprKind::Binary { op, lhs, rhs } => format!(
                "({} {} {})",
                op.symbol(),
                self.render_expr(*lhs),
                self.render_expr(*rhs)
            ),
            ExprKind::Index { base, index } => {
                format!(
                    "(index {} {})",
                    self.render_expr(*base),
                    self.render_expr(*index)
                )
            }
            ExprKind::Field { base, name } => format!("(. {} {})", self.render_expr(*base), name),
            ExprKind::Call { callee, args } => {
                let mut out = format!("(call {}", callee);
                for a in args {
                    out.push(' ');
                    if a.is_mut {
                        out.push_str("mut ");
                    }
                    out.push_str(&self.render_expr(a.value));
                }
                out.push(')');
                out
            }
            ExprKind::Array(items) => {
                let inner: Vec<String> = items.iter().map(|e| self.render_expr(*e)).collect();
                format!("[{}]", inner.join(" "))
            }
            ExprKind::Repeat { value, count } => {
                format!("[{}; {}]", self.render_expr(*value), count)
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let mut out = format!(
                    "(if {} {}",
                    self.render_expr(*cond),
                    self.render_block(*then)
                );
                match otherwise {
                    Some(Else::Block(b)) => {
                        out.push(' ');
                        out.push_str(&self.render_block(*b));
                    }
                    Some(Else::If(e)) => {
                        out.push(' ');
                        out.push_str(&self.render_expr(*e));
                    }
                    None => {}
                }
                out.push(')');
                out
            }
            ExprKind::Error => String::from("<error>"),
        }
    }

    /// A block in canonical form: `{s1; s2}`.
    pub fn render_block(&self, id: BlockId) -> String {
        let inner: Vec<String> = self
            .block(id)
            .stmts
            .iter()
            .map(|s| self.render_stmt(*s))
            .collect();
        format!("{{{}}}", inner.join("; "))
    }

    /// A statement in canonical form.
    pub fn render_stmt(&self, id: StmtId) -> String {
        match &self.stmt(id).kind {
            StmtKind::Binding {
                is_var,
                name,
                ty,
                init,
                ..
            } => {
                let kw = if *is_var { "var" } else { "val" };
                match ty {
                    Some(t) => format!(
                        "({} {} {} {})",
                        kw,
                        name,
                        self.render_type(*t),
                        self.render_expr(*init)
                    ),
                    None => format!("({} {} {})", kw, name, self.render_expr(*init)),
                }
            }
            StmtKind::Assign { place, op, value } => {
                let sym = match op {
                    AssignOp::Set => String::from("="),
                    AssignOp::Compound(b) => format!("{}=", b.symbol()),
                };
                format!(
                    "({} {} {})",
                    sym,
                    self.render_expr(*place),
                    self.render_expr(*value)
                )
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                let mut out = format!("(while {}", self.render_expr(*cond));
                for i in invariants {
                    out.push_str(&format!(" (invariant {})", self.render_expr(*i)));
                }
                for d in decreases {
                    out.push_str(&format!(" (decreases {})", self.render_expr(*d)));
                }
                out.push_str(&format!(" {})", self.render_block(*body)));
                out
            }
            StmtKind::Guard { cond, otherwise } => format!(
                "(guard {} {})",
                self.render_expr(*cond),
                self.render_block(*otherwise)
            ),
            StmtKind::Return(Some(e)) => format!("(return {})", self.render_expr(*e)),
            StmtKind::Return(None) => String::from("(return)"),
            StmtKind::Break => String::from("break"),
            StmtKind::Continue => String::from("continue"),
            StmtKind::Discard(e) => format!("(discard {})", self.render_expr(*e)),
            StmtKind::Expr(e) => self.render_expr(*e),
        }
    }

    /// A type in canonical form: `u8[0..10]`, `[4]bool`.
    pub fn render_type(&self, id: TypeId) -> String {
        match &self.ty(id).kind {
            TypeKind::Int { name, refinement } => match refinement {
                Some(r) => format!(
                    "{}[{}{}{}]",
                    name,
                    render_bound(&r.lo),
                    if r.inclusive { "..=" } else { ".." },
                    render_bound(&r.hi)
                ),
                None => name.clone(),
            },
            TypeKind::Bool => String::from("bool"),
            TypeKind::Array { len, elem } => format!("[{}]{}", len, self.render_type(*elem)),
            TypeKind::Error => String::from("<error>"),
        }
    }
}

fn render_bound(b: &Bound) -> String {
    match b {
        Bound::Int { negative, value } => {
            if *negative {
                format!("-{}", value)
            } else {
                value.to_string()
            }
        }
        Bound::Name(n) => n.clone(),
        Bound::Len(n) => format!("{}.len", n),
    }
}

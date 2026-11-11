//! The syntax tree of one function (`spec/grammar.md`, items 4 to 7, and the parts of items 10
//! to 12 that M2's structs bring), and of one struct declaration.
//!
//! The nodes of a function live in the tables of its [`Tree`] and refer to each other by index,
//! as `MirRegionSoA` does, so the tree has no pointers and no lifetimes. A node keeps the span
//! of the text it was parsed from. [`Tree::render_expr`] prints an expression in one canonical
//! form, fully parenthesised, which the tests compare against. A struct declaration keeps its
//! fields' types in a tree of its own ([`StructSyntax`]).

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

/// A field's value where a struct is built: `name: value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInit {
    pub name: String,
    pub name_span: Span,
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
    /// `a.name`: an array's `len`, or a struct's field.
    Field {
        base: ExprId,
        name: String,
    },
    /// `f(a, b)`, whose arguments are positional.
    Call {
        callee: String,
        args: Vec<Arg>,
    },
    /// `Name(field: value, ...)`, whose arguments are named: a struct being built.
    Construct {
        name: String,
        fields: Vec<FieldInit>,
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
    /// A type named by a declaration of the module: a struct.
    Named(String),
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

/// One field of a struct declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSyntax {
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
    pub ty: TypeId,
}

/// One struct declaration (`grammar.md`, item 10): its fields in the order written, their types
/// in its own tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructSyntax {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<FieldSyntax>,
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

    /// The node of the greatest height, an expression, a block or a type counting one level
    /// each, and its height, when the tree is higher than `limit`: the node at exactly
    /// `limit + 1`, the innermost that goes too deep. Computed without recursion, since it
    /// guards every phase that recurses over the tree.
    pub fn too_deep(&self, limit: u32) -> Option<Span> {
        #[derive(Clone, Copy)]
        enum Node {
            E(u32),
            S(u32),
            B(u32),
            T(u32),
        }
        let mut heights: [Vec<Option<u32>>; 4] = [
            vec![None; self.exprs.len()],
            vec![None; self.stmts.len()],
            vec![None; self.blocks.len()],
            vec![None; self.types.len()],
        ];
        let slot = |n: Node| match n {
            Node::E(i) => (0, i as usize),
            Node::S(i) => (1, i as usize),
            Node::B(i) => (2, i as usize),
            Node::T(i) => (3, i as usize),
        };
        let children = |n: Node, out: &mut Vec<Node>| {
            out.clear();
            match n {
                Node::E(i) => match &self.exprs[i as usize].kind {
                    ExprKind::Unary { operand, .. } => out.push(Node::E(operand.0)),
                    ExprKind::Binary { lhs, rhs, .. } => {
                        out.extend([Node::E(lhs.0), Node::E(rhs.0)]);
                    }
                    ExprKind::Index { base, index } => {
                        out.extend([Node::E(base.0), Node::E(index.0)]);
                    }
                    ExprKind::Field { base, .. } => out.push(Node::E(base.0)),
                    ExprKind::Call { args, .. } => {
                        out.extend(args.iter().map(|a| Node::E(a.value.0)));
                    }
                    ExprKind::Construct { fields, .. } => {
                        out.extend(fields.iter().map(|f| Node::E(f.value.0)));
                    }
                    ExprKind::Array(items) => out.extend(items.iter().map(|e| Node::E(e.0))),
                    ExprKind::Repeat { value, .. } => out.push(Node::E(value.0)),
                    ExprKind::If {
                        cond,
                        then,
                        otherwise,
                    } => {
                        out.extend([Node::E(cond.0), Node::B(then.0)]);
                        match otherwise {
                            Some(Else::Block(b)) => out.push(Node::B(b.0)),
                            Some(Else::If(e)) => out.push(Node::E(e.0)),
                            None => {}
                        }
                    }
                    _ => {}
                },
                Node::S(i) => match &self.stmts[i as usize].kind {
                    StmtKind::Binding { ty, init, .. } => {
                        out.extend(ty.map(|t| Node::T(t.0)));
                        out.push(Node::E(init.0));
                    }
                    StmtKind::Assign { place, value, .. } => {
                        out.extend([Node::E(place.0), Node::E(value.0)]);
                    }
                    StmtKind::While {
                        cond,
                        invariants,
                        decreases,
                        body,
                    } => {
                        out.push(Node::E(cond.0));
                        out.extend(invariants.iter().chain(decreases).map(|e| Node::E(e.0)));
                        out.push(Node::B(body.0));
                    }
                    StmtKind::Guard { cond, otherwise } => {
                        out.extend([Node::E(cond.0), Node::B(otherwise.0)]);
                    }
                    StmtKind::Return(Some(e)) | StmtKind::Discard(e) | StmtKind::Expr(e) => {
                        out.push(Node::E(e.0));
                    }
                    _ => {}
                },
                Node::B(i) => {
                    out.extend(self.blocks[i as usize].stmts.iter().map(|s| Node::S(s.0)))
                }
                Node::T(i) => {
                    if let TypeKind::Array { elem, .. } = &self.types[i as usize].kind {
                        out.push(Node::T(elem.0));
                    }
                }
            }
        };
        let span = |n: Node| match n {
            Node::E(i) => self.exprs[i as usize].span,
            Node::S(i) => self.stmts[i as usize].span,
            Node::B(i) => self.blocks[i as usize].span,
            Node::T(i) => self.types[i as usize].span,
        };
        let roots = (0..self.exprs.len() as u32)
            .map(Node::E)
            .chain((0..self.stmts.len() as u32).map(Node::S))
            .chain((0..self.blocks.len() as u32).map(Node::B))
            .chain((0..self.types.len() as u32).map(Node::T));
        let mut found = None;
        let mut stack: Vec<(Node, bool)> = Vec::new();
        let mut kids = Vec::new();
        for root in roots {
            let (k, i) = slot(root);
            if heights[k][i].is_some() {
                continue;
            }
            stack.push((root, false));
            while let Some((n, expanded)) = stack.pop() {
                let (k, i) = slot(n);
                if heights[k][i].is_some() {
                    continue;
                }
                children(n, &mut kids);
                if expanded {
                    let below = kids
                        .iter()
                        .filter_map(|c| {
                            let (ck, ci) = slot(*c);
                            heights[ck][ci]
                        })
                        .max()
                        .unwrap_or(0);
                    // A statement only wraps what it holds; the rest are levels.
                    let h = below + u32::from(!matches!(n, Node::S(_)));
                    heights[k][i] = Some(h);
                    if h == limit + 1 && found.is_none() {
                        found = Some(span(n));
                    }
                } else {
                    stack.push((n, true));
                    for c in kids.clone() {
                        let (ck, ci) = slot(c);
                        if heights[ck][ci].is_none() {
                            stack.push((c, false));
                        }
                    }
                }
            }
        }
        found
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
    /// A function in its canonical form: `(fn name (params) result (clauses) body)`, where a
    /// parameter is `(name type)` or `(mut name type)`, a missing result `()`, a clause
    /// `(requires e)`, `(ensures e)` or `(decreases e)`, and a missing body `()`.
    pub fn render_fn(f: &FnSyntax) -> String {
        let tree = &f.tree;
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let m = if p.is_mut { "mut " } else { "" };
                format!("({}{} {})", m, p.name, tree.render_type(p.ty))
            })
            .collect();
        let result = f.result.map_or("()".to_string(), |t| tree.render_type(t));
        let clauses: Vec<String> = f
            .contracts
            .iter()
            .map(|c| {
                let kind = match c.kind {
                    ContractKind::Requires => "requires",
                    ContractKind::Ensures => "ensures",
                    ContractKind::Decreases => "decreases",
                };
                format!("({} {})", kind, tree.render_expr(c.expr))
            })
            .collect();
        let body = match f.body {
            Some(Body::Block(b)) => tree.render_block(b),
            Some(Body::Expr(e)) => format!("(= {})", tree.render_expr(e)),
            None => "()".to_string(),
        };
        format!(
            "(fn {} ({}) {} ({}) {})",
            f.name,
            params.join(" "),
            result,
            clauses.join(" "),
            body
        )
    }

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
            ExprKind::Construct { name, fields } => {
                let mut out = format!("(new {}", name);
                for f in fields {
                    out.push_str(&format!(" ({} {})", f.name, self.render_expr(f.value)));
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
            TypeKind::Named(name) => name.clone(),
            TypeKind::Error => String::from("<error>"),
        }
    }

    /// A struct declaration in its canonical form: `(struct Name (field type) ...)`, a public
    /// field as `(pub field type)`.
    pub fn render_struct(s: &StructSyntax) -> String {
        let fields: Vec<String> = s
            .fields
            .iter()
            .map(|f| {
                let p = if f.is_pub { "pub " } else { "" };
                format!("({}{} {})", p, f.name, s.tree.render_type(f.ty))
            })
            .collect();
        format!("(struct {} {})", s.name, fields.join(" "))
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

#[cfg(test)]
mod tests {
    /// A function whose body is `body`, as the module holds it, and its syntax tree.
    fn parsed(body: &str) -> (String, crate::tree::FnSyntax) {
        let src = format!("module m\npub fn d(x: i32) -> i32 {}\n", body);
        let module = crate::parse_module(&src).unwrap();
        let p = crate::parse_bodies(&src, &module).remove(0);
        assert!(p.errors.is_empty(), "{}: {:?}", body, p.errors);
        (src, p.syntax.unwrap())
    }

    /// The tree's height as [`Tree::too_deep`](super::Tree::too_deep) measures it: the least
    /// limit it finds nothing past.
    fn height(body: &str) -> u32 {
        let (_, f) = parsed(body);
        (0..).find(|&l| f.tree.too_deep(l).is_none()).unwrap()
    }

    /// Each kind of node counts what it holds, an expression, a block or a type one level and a
    /// statement none, and the node reported is the one at exactly a level past the limit.
    #[test]
    fn test_the_height_counts_every_node_kind_and_reports_the_first_level_past() {
        // `x[0][0][0]`: three indexes above `x`.
        const C: &str = "x[0][0][0]";
        assert_eq!(height(&format!("= {}", C)), 4);
        for (body, levels) in [
            ("= -C", 1),
            ("= C.len", 1),
            ("= f(C)", 1),
            ("= [C]", 1),
            ("= [C; 2]", 1),
            // The condition's comparison, then the `if`.
            ("= if C > 0 { 1 } else { 2 }", 2),
            ("= if x > 0 { 1 } else if C > 0 { 2 } else { 3 }", 3),
            // The block, then the statement, which adds none.
            ("{\n  val y: i32 = C\n  0\n}", 1),
            ("{\n  y = C\n  0\n}", 1),
            ("{\n  while C > 0 {\n  }\n  0\n}", 2),
            ("{\n  guard C > 0 else {\n    return 0\n  }\n  0\n}", 2),
            ("{\n  return C\n}", 1),
            ("{\n  _ = C\n  0\n}", 1),
            ("{\n  C\n}", 1),
        ] {
            let body = body.replace('C', C);
            assert_eq!(height(&body), 4 + levels, "{}", body);
        }
        let (src, f) = parsed(&format!("= {}", C));
        let at = f.tree.too_deep(3).unwrap();
        assert_eq!(&src[at.start..at.end], C);
        assert!(f.tree.too_deep(4).is_none());
    }
}

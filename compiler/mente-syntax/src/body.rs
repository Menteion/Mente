//! The parser of M1's functions (`spec/grammar.md`, items 2 to 8).
//!
//! [`parse_function`] reads one function from its tokens: the header, the contracts and the
//! body, into a [`FnSyntax`]. Expressions are parsed by precedence climbing (Pratt), with the
//! binding powers of `grammar.md`, item 7. A newline ends a statement where item 2 says so: the
//! parser checks for one before every infix and postfix operator, outside parentheses and
//! brackets, and never right after a token that cannot end a statement.
//!
//! Every error is a [`Diagnostic`]. After one, the parser skips to the next statement, so one
//! mistake yields one diagnostic. A construct of a later milestone is reported as such
//! (`E0109`), naming the milestone that brings it.

use crate::diagnostics::{Applicability, Diagnostic};
use crate::tree::{
    Arg, AssignOp, BinaryOp, Block, BlockId, Body, Bound, Contract, ContractKind, Else, ExprId,
    ExprKind, FnSyntax, Param, Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use crate::{Span, Token, TokenKind};

/// The integer types of M1 (`grammar.md`, item 5).
pub const INT_TYPES: [&str; 11] = [
    "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "usize",
];

/// Parses one function from its tokens, `fn` to the end of its body, which `src` holds at the
/// tokens' spans.
pub fn parse_function(src: &str, tokens: &[Token]) -> (FnSyntax, Vec<Diagnostic>) {
    let mut tokens = tokens.to_vec();
    let last = tokens.last().map(|t| t.span).unwrap_or_default();
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: last.end,
            end: last.end,
            line: last.line,
            col: last.col,
        },
    });
    let base = tokens[0].span.start;
    let mut p = Parser {
        text: src[base..last.end.max(base)].to_string(),
        base,
        tokens,
        pos: 0,
        tree: Tree::default(),
        errors: Vec::new(),
        recovering: false,
        nest: 0,
    };
    let syntax = p.function();
    (syntax, p.errors)
}

/// Where a later milestone brings a construct; `None` when no milestone has it yet.
type Milestone = Option<&'static str>;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// The function's source text, and where it starts in the module.
    text: String,
    base: usize,
    tree: Tree,
    errors: Vec<Diagnostic>,
    /// After an error, until the parser has skipped to the next statement.
    recovering: bool,
    /// How many `(` and `[` are open, inside which a newline is whitespace.
    nest: u32,
}

impl Parser {
    // ---------------------------------------------------------------- tokens

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    /// The token after the next one, or the end.
    fn peek_second(&self) -> &TokenKind {
        &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)].kind
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    /// The end of the last token taken.
    fn prev_end(&self) -> usize {
        self.tokens[self.pos.saturating_sub(1)].span.end
    }

    /// Whether a newline stands between the last token taken and the next one.
    fn newline_before(&self) -> bool {
        self.tokens[self.pos].span.line > self.tokens[self.pos.saturating_sub(1)].span.line
    }

    /// Whether the statement being read ends before the next token (`grammar.md`, item 2).
    fn ends_here(&self) -> bool {
        self.nest == 0 && self.newline_before()
    }

    fn text_of(&self, span: Span) -> &str {
        &self.text[span.start - self.base..span.end - self.base]
    }

    /// From the start of `start` to the end of the last token taken.
    fn span_from(&self, start: Span) -> Span {
        Span {
            end: self.prev_end().max(start.end),
            ..start
        }
    }

    /// The next token, as an error message names it.
    fn found(&self) -> String {
        match self.peek() {
            TokenKind::Eof => String::from("the end of the function"),
            _ => format!("`{}`", self.text_of(self.span())),
        }
    }

    // ---------------------------------------------------------------- errors

    /// Reports `d`, unless the parser is already recovering from an error.
    fn error(&mut self, d: Diagnostic) {
        if !self.recovering {
            self.errors.push(d);
        }
        self.recovering = true;
    }

    fn expected(&mut self, what: &str) {
        let d = Diagnostic::error(
            "E0111",
            self.span(),
            format!("expected {}, found {}", what, self.found()),
        );
        self.error(d);
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> bool {
        let found = self.eat(kind);
        if !found {
            self.expected(what);
        }
        found
    }

    /// Reports a construct of a later milestone (`E0109`).
    fn later(&mut self, span: Span, what: &str, milestone: Milestone) {
        let message = match milestone {
            Some(m) => format!("{} comes in {}; M1 does not have it", what, m),
            None => format!("{} is in no milestone yet; M1 does not have it", what),
        };
        self.error(Diagnostic::error("E0109", span, message));
    }

    /// Skips the rest of the statement after an error: to the end of its line, past any
    /// brackets it opened, or to the `}` that closes the enclosing block.
    fn skip_statement(&mut self) {
        let mut depth = 0i32;
        let start = self.pos;
        loop {
            match self.peek() {
                TokenKind::Eof => break,
                TokenKind::RBrace if depth <= 0 => break,
                _ if depth <= 0 && self.pos > start && self.newline_before() => break,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
                _ => {}
            }
            self.bump();
        }
        self.recovering = false;
    }

    /// A name: an identifier, never a keyword (`E0101`).
    fn name(&mut self, what: &str) -> (String, Span) {
        let span = self.span();
        if let TokenKind::Ident(n) = self.peek().clone() {
            self.bump();
            return (n, span);
        }
        let d = Diagnostic::error(
            "E0101",
            span,
            format!("expected {}, found {}", what, self.found()),
        );
        self.error(d);
        (String::new(), span)
    }

    // ---------------------------------------------------------------- the function

    fn function(&mut self) -> FnSyntax {
        self.bump(); // `fn`
        let (name, name_span) = self.name("a function name");
        if self.at(&TokenKind::Lt) {
            let span = self.span();
            self.later(span, "a type parameter list", Some("M4"));
        }
        if !self.recovering {
            self.expect(&TokenKind::LParen, "`(` and the parameters");
        }
        let mut params = Vec::new();
        if !self.recovering {
            self.nest += 1;
            while !self.at(&TokenKind::RParen) && !self.recovering {
                let before = self.pos;
                params.push(self.param());
                if self.pos == before {
                    break;
                }
                if !self.eat(&TokenKind::Comma) && !self.at(&TokenKind::RParen) {
                    self.expected("`,` or `)`");
                }
            }
            self.nest -= 1;
            self.eat(&TokenKind::RParen);
        }
        self.skip_to_clauses();

        let mut result = None;
        if self.eat(&TokenKind::Arrow) {
            result = Some(self.ty());
            if self.at(&TokenKind::Bang) {
                let span = self.span();
                self.later(span, "an error type `T ! E`", Some("M2"));
            }
        }
        if matches!(self.peek(), TokenKind::Ident(w) if w == "from") {
            let span = self.span();
            self.later(span, "a `from` clause", Some("M2"));
        }
        if self.at(&TokenKind::With) {
            let span = self.span();
            self.later(span, "an effect clause `with`", Some("M4"));
        }
        self.skip_to_clauses();

        let mut contracts = Vec::new();
        loop {
            let kind = match self.peek() {
                TokenKind::Requires => ContractKind::Requires,
                TokenKind::Ensures => ContractKind::Ensures,
                TokenKind::Decreases => ContractKind::Decreases,
                _ => break,
            };
            self.bump();
            let expr = self.expr();
            contracts.push(Contract { kind, expr });
            self.skip_to_clauses();
        }

        let body = match self.peek() {
            TokenKind::LBrace => Some(Body::Block(self.block())),
            TokenKind::Eq => {
                self.bump();
                let e = self.expr();
                if !self.at(&TokenKind::Eof) {
                    self.expected("the end of the expression body");
                }
                Some(Body::Expr(e))
            }
            TokenKind::Eof => None,
            _ => {
                self.expected("`{` or `=` and the function's body");
                None
            }
        };
        FnSyntax {
            name,
            name_span,
            params,
            result,
            contracts,
            body,
            tree: std::mem::take(&mut self.tree),
        }
    }

    /// After an error in the header, skips to its next clause or to the body.
    fn skip_to_clauses(&mut self) {
        if !self.recovering {
            return;
        }
        while !matches!(
            self.peek(),
            TokenKind::Requires
                | TokenKind::Ensures
                | TokenKind::Decreases
                | TokenKind::LBrace
                | TokenKind::Eq
                | TokenKind::Eof
        ) {
            self.bump();
        }
        self.recovering = false;
    }

    fn param(&mut self) -> Param {
        let is_mut = self.eat(&TokenKind::Mut);
        if matches!(self.peek(), TokenKind::Own | TokenKind::In) {
            let span = self.span();
            let what = format!("an `{}` parameter", self.text_of(span));
            self.later(span, &what, Some("M2"));
        }
        let (name, name_span) = self.name("a parameter name");
        let ty = if self.recovering {
            self.tree.push_type(TypeKind::Error, name_span)
        } else if self.expect(&TokenKind::Colon, "`:` and the parameter's type") {
            self.ty()
        } else {
            self.tree.push_type(TypeKind::Error, name_span)
        };
        Param {
            is_mut,
            name,
            name_span,
            ty,
        }
    }

    // ---------------------------------------------------------------- types

    fn ty(&mut self) -> TypeId {
        let start = self.span();
        match self.peek().clone() {
            TokenKind::Ident(n) if INT_TYPES.contains(&n.as_str()) => {
                self.bump();
                let refinement = if self.at(&TokenKind::LBracket) && !self.newline_before() {
                    self.refinement()
                } else {
                    None
                };
                let span = self.span_from(start);
                self.tree.push_type(
                    TypeKind::Int {
                        name: n,
                        refinement,
                    },
                    span,
                )
            }
            TokenKind::Ident(n) if n == "bool" => {
                self.bump();
                self.tree.push_type(TypeKind::Bool, start)
            }
            TokenKind::LBracket => {
                self.bump();
                if self.at(&TokenKind::RBracket) {
                    self.later(start, "a slice type `[]T`", Some("M2"));
                    return self.tree.push_type(TypeKind::Error, start);
                }
                let len = match self.peek().clone() {
                    TokenKind::IntLit(v, _) => {
                        self.bump();
                        v
                    }
                    _ => {
                        self.bad_type(
                            "an array's length is an integer literal, as in `[4]u8`".into(),
                        );
                        return self.tree.push_type(TypeKind::Error, start);
                    }
                };
                if !self.expect(&TokenKind::RBracket, "`]` after the array's length") {
                    return self.tree.push_type(TypeKind::Error, start);
                }
                let elem = self.ty();
                let span = self.span_from(start);
                self.tree.push_type(TypeKind::Array { len, elem }, span)
            }
            TokenKind::Question => {
                self.later(start, "an optional type `?T`", Some("M2"));
                self.tree.push_type(TypeKind::Error, start)
            }
            TokenKind::Ident(n) => {
                let milestone = match n.as_str() {
                    "str" | "String" | "List" => Some(("M2", "a string or list type")),
                    "f32" | "f64" | "dec" | "sat" | "wrap" => {
                        Some(("M5", "a floating-point, decimal or saturating type"))
                    }
                    "Arena" | "Pool" | "Handle" | "Local" | "Shared" | "SoA" | "Tiled" => {
                        Some(("M3", "an arena, pool or layout type"))
                    }
                    _ => None,
                };
                match milestone {
                    Some((m, what)) => {
                        let what = format!("{} (`{}`)", what, n);
                        self.later(start, &what, Some(m));
                    }
                    None => self.bad_type(format!(
                        "`{}` is not a type of M1, whose types are the integer types, `bool` and arrays `[N]T`; structs and enums come in M2",
                        n
                    )),
                }
                self.tree.push_type(TypeKind::Error, start)
            }
            _ => {
                self.bad_type(format!("expected a type, found {}", self.found()));
                self.tree.push_type(TypeKind::Error, start)
            }
        }
    }

    fn bad_type(&mut self, message: String) {
        let d = Diagnostic::error("E0116", self.span(), message);
        self.error(d);
    }

    /// `[lo..hi]` or `[lo..=hi]` after an integer type.
    fn refinement(&mut self) -> Option<Refinement> {
        self.bump(); // `[`
        let lo = self.bound()?;
        let inclusive = match self.peek() {
            TokenKind::DotDot => false,
            TokenKind::DotDotEq => true,
            _ => {
                self.expected("`..` or `..=` between the refinement's bounds");
                return None;
            }
        };
        self.bump();
        let hi = self.bound()?;
        if !self.expect(&TokenKind::RBracket, "`]` to close the refinement") {
            return None;
        }
        Some(Refinement { lo, hi, inclusive })
    }

    fn bound(&mut self) -> Option<Bound> {
        let negative = self.eat(&TokenKind::Minus);
        match self.peek().clone() {
            TokenKind::IntLit(value, _) => {
                self.bump();
                Some(Bound::Int { negative, value })
            }
            TokenKind::Ident(n) if !negative => {
                self.bump();
                if self.at(&TokenKind::Dot) && self.peek_second() == &TokenKind::Ident("len".into())
                {
                    self.bump();
                    self.bump();
                    return Some(Bound::Len(n));
                }
                Some(Bound::Name(n))
            }
            _ => {
                self.bad_type(format!(
                    "a refinement's bound is an integer literal, a name, or `name.len`, not {}",
                    self.found()
                ));
                None
            }
        }
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self) -> BlockId {
        let open = self.span();
        if !self.expect(&TokenKind::LBrace, "`{`") {
            return self.tree.push_block(Block {
                stmts: Vec::new(),
                span: open,
            });
        }
        let saved_nest = self.nest;
        self.nest = 0;
        let mut stmts = Vec::new();
        loop {
            match self.peek() {
                TokenKind::RBrace => {
                    self.bump();
                    break;
                }
                TokenKind::Eof => {
                    let d = Diagnostic::error("E0111", open, "this block has no closing `}`");
                    self.error(d);
                    break;
                }
                _ => {}
            }
            let before = self.pos;
            stmts.push(self.stmt());
            if self.recovering {
                self.skip_statement();
            } else {
                self.statement_end();
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.nest = saved_nest;
        let span = self.span_from(open);
        self.tree.push_block(Block { stmts, span })
    }

    /// After a statement, the next one starts on a new line (`E0112`).
    fn statement_end(&mut self) {
        if matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) || self.newline_before() {
            return;
        }
        let at = self.span();
        let gap = Span {
            start: self.prev_end(),
            end: at.start,
            ..at
        };
        if self.at(&TokenKind::Semi) {
            self.bump();
            let last_on_line =
                matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) || self.newline_before();
            let d = Diagnostic::error(
                "E0112",
                at,
                "`;` does not end a statement in Mente; a newline does",
            );
            let d = if last_on_line {
                d.with_fix(Applicability::MachineApplicable, "remove the `;`", at, "")
            } else {
                let to_next = Span {
                    end: self.span().start,
                    ..at
                };
                d.with_fix(
                    Applicability::MachineApplicable,
                    "start the next statement on a new line",
                    to_next,
                    "\n",
                )
            };
            self.errors.push(d);
            return;
        }
        let d = Diagnostic::error(
            "E0112",
            at,
            format!(
                "a statement ends at the end of its line, and {} starts another on the same line",
                self.found()
            ),
        )
        .with_fix(
            Applicability::MachineApplicable,
            "start it on a new line",
            gap,
            "\n",
        );
        self.error(d);
        self.skip_statement();
    }

    fn stmt(&mut self) -> StmtId {
        let start = self.span();
        let kind = match self.peek().clone() {
            TokenKind::Val | TokenKind::Var => self.binding(),
            TokenKind::While => self.while_loop(),
            TokenKind::Guard => {
                self.bump();
                let cond = self.cond();
                self.expect(
                    &TokenKind::Else,
                    "`else` and a block after the guard's condition",
                );
                let otherwise = self.block();
                StmtKind::Guard { cond, otherwise }
            }
            TokenKind::Return => {
                self.bump();
                let value = if matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof)
                    || self.newline_before()
                {
                    None
                } else {
                    Some(self.expr())
                };
                StmtKind::Return(value)
            }
            TokenKind::Break => {
                self.bump();
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.bump();
                StmtKind::Continue
            }
            TokenKind::For => {
                self.later(start, "a `for` loop", None);
                StmtKind::Break
            }
            _ => {
                let target = self.expr();
                match assign_op(self.peek()) {
                    Some(op) if !self.recovering && !self.ends_here() => {
                        let op_span = self.span();
                        if !self.is_place(target) {
                            let place = self.tree.expr(target).span;
                            let d = Diagnostic::error(
                                "E0114",
                                place,
                                format!(
                                    "`{}` is not a place to assign to; a place is a name, or an element of one such as `a[i]`",
                                    self.text_of(place)
                                ),
                            )
                            .with_related(op_span, "assigned here");
                            self.error(d);
                        }
                        self.bump();
                        let value = self.expr();
                        StmtKind::Assign {
                            place: target,
                            op,
                            value,
                        }
                    }
                    _ => StmtKind::Expr(target),
                }
            }
        };
        let span = self.span_from(start);
        self.tree.push_stmt(kind, span)
    }

    fn binding(&mut self) -> StmtKind {
        let is_var = self.bump().kind == TokenKind::Var;
        let (name, name_span) = self.name("a name for the binding");
        let ty = if !self.recovering && self.eat(&TokenKind::Colon) {
            Some(self.ty())
        } else {
            None
        };
        let init = if !self.recovering && self.expect(&TokenKind::Eq, "`=` and the binding's value")
        {
            self.expr()
        } else {
            self.tree.push_expr(ExprKind::Error, name_span)
        };
        StmtKind::Binding {
            is_var,
            name,
            name_span,
            ty,
            init,
        }
    }

    fn while_loop(&mut self) -> StmtKind {
        self.bump();
        let cond = self.cond();
        let mut invariants = Vec::new();
        let mut decreases = Vec::new();
        loop {
            match self.peek() {
                TokenKind::Invariant => {
                    self.bump();
                    invariants.push(self.cond());
                }
                TokenKind::Decreases => {
                    self.bump();
                    decreases.push(self.cond());
                }
                _ => break,
            }
        }
        let body = self.block();
        StmtKind::While {
            cond,
            invariants,
            decreases,
            body,
        }
    }

    /// A condition or a clause: an expression that is not itself an `if`.
    fn cond(&mut self) -> ExprId {
        self.binary(0)
    }

    fn is_place(&self, e: ExprId) -> bool {
        match &self.tree.expr(e).kind {
            ExprKind::Name(_) => true,
            ExprKind::Index { base, .. } => self.is_place(*base),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- expressions

    /// A full expression, which may be an `if` (`grammar.md`, item 7).
    fn expr(&mut self) -> ExprId {
        if !self.at(&TokenKind::If) {
            return self.binary(0);
        }
        let e = self.if_expr();
        if !self.recovering && !self.ends_here() && infix(self.peek()).is_some() {
            self.if_in_operand(e);
        }
        e
    }

    /// An `if` used as an operand needs parentheses (`E0115`); the fix adds them.
    fn if_in_operand(&mut self, e: ExprId) {
        let span = self.tree.expr(e).span;
        let wrapped = format!("({})", self.text_of(span));
        let d = Diagnostic::error(
            "E0115",
            span,
            "an `if` expression used as an operand needs parentheses",
        )
        .with_fix(
            Applicability::MachineApplicable,
            "put the `if` expression in parentheses",
            span,
            wrapped,
        );
        self.error(d);
    }

    fn binary(&mut self, min_bp: u8) -> ExprId {
        let start = self.span();
        let mut lhs = self.prefix();
        loop {
            if self.recovering || self.ends_here() {
                break;
            }
            if self.postfix(&mut lhs, start) {
                continue;
            }
            if self.recovering {
                break;
            }
            let Some((op, lbp)) = infix(self.peek()) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.binary(lbp + 1);
            if is_comparison(op) && !self.recovering && !self.ends_here() {
                if let Some((next, _)) = infix(self.peek()) {
                    if is_comparison(next) {
                        self.chained(rhs);
                    }
                }
            }
            let span = self.span_from(start);
            lhs = self.tree.push_expr(ExprKind::Binary { op, lhs, rhs }, span);
        }
        lhs
    }

    /// `a < b < c`: comparisons do not chain (`E0113`).
    fn chained(&mut self, middle: ExprId) {
        let b = self.tree.expr(middle).span;
        let middle_text = self.text_of(b).to_string();
        let d = Diagnostic::error(
            "E0113",
            self.span(),
            format!(
                "comparisons do not chain; write `... {} and {} ...`",
                middle_text, middle_text
            ),
        )
        .with_fix(
            Applicability::Suggestion,
            format!("compare `{}` twice, joined by `and`", middle_text),
            Span { start: b.end, ..b },
            format!(" and {}", middle_text),
        );
        self.error(d);
    }

    /// Applies one postfix operator to `lhs`, if the next token is one.
    fn postfix(&mut self, lhs: &mut ExprId, start: Span) -> bool {
        let at = self.span();
        match self.peek() {
            TokenKind::LBracket => {
                self.bump();
                if self.at(&TokenKind::Mut) {
                    let span = self.span();
                    self.later(span, "a `mut` projection `p[mut h]`", Some("M3"));
                    return true;
                }
                self.nest += 1;
                let index = self.expr();
                if !self.recovering {
                    self.expect(&TokenKind::RBracket, "`]` to close the index");
                }
                self.nest -= 1;
                let span = self.span_from(start);
                *lhs = self
                    .tree
                    .push_expr(ExprKind::Index { base: *lhs, index }, span);
                true
            }
            TokenKind::Dot => {
                self.bump();
                let (name, name_span) = self.name("a field name after `.`");
                if self.recovering {
                    return true;
                }
                if self.at(&TokenKind::LParen) {
                    self.later(name_span, "a method call", Some("M2"));
                } else if name != "len" {
                    self.later(name_span, "a field other than `.len`", Some("M2"));
                } else {
                    let span = self.span_from(start);
                    *lhs = self
                        .tree
                        .push_expr(ExprKind::Field { base: *lhs, name }, span);
                }
                true
            }
            TokenKind::Question | TokenKind::Bang => {
                self.later(at, "error propagation (`?` and `!`)", Some("M2"));
                true
            }
            TokenKind::QuestionQuestion | TokenKind::QuestionDot => {
                self.later(
                    at,
                    "an optional value's operator (`??` or `?.`)",
                    Some("M2"),
                );
                true
            }
            TokenKind::PipeGt => {
                self.later(at, "the pipe `|>`", None);
                true
            }
            TokenKind::DotDot | TokenKind::DotDotEq => {
                self.later(at, "a range in an expression", None);
                true
            }
            _ => false,
        }
    }

    fn prefix(&mut self) -> ExprId {
        let start = self.span();
        let kind = match self.peek().clone() {
            TokenKind::IntLit(value, raw) => {
                self.bump();
                ExprKind::Int {
                    value,
                    suffix: int_suffix(&raw),
                }
            }
            TokenKind::TrueLit => {
                self.bump();
                ExprKind::Bool(true)
            }
            TokenKind::FalseLit => {
                self.bump();
                ExprKind::Bool(false)
            }
            TokenKind::Ident(n) => {
                self.bump();
                if self.at(&TokenKind::LParen) && !self.newline_before() {
                    if n == "todo" {
                        self.todo()
                    } else {
                        self.call(n)
                    }
                } else {
                    ExprKind::Name(n)
                }
            }
            TokenKind::LParen => {
                self.bump();
                self.nest += 1;
                let inner = self.expr();
                if !self.recovering {
                    self.expect(&TokenKind::RParen, "`)` to close the parenthesis");
                }
                self.nest -= 1;
                return inner;
            }
            TokenKind::LBracket => self.array(),
            TokenKind::Minus => self.unary(UnaryOp::Neg, UNARY_OPERAND_BP),
            TokenKind::Tilde => self.unary(UnaryOp::BitNot, UNARY_OPERAND_BP),
            TokenKind::Not => self.unary(UnaryOp::Not, NOT_OPERAND_BP),
            TokenKind::If => {
                let e = self.if_expr();
                self.if_in_operand(e);
                return e;
            }
            TokenKind::StrLit(_) => {
                self.later(start, "a string value", Some("M2"));
                ExprKind::Error
            }
            TokenKind::FloatLit(_) => {
                self.later(start, "a floating-point or fixed-point value", Some("M5"));
                ExprKind::Error
            }
            TokenKind::NoneLit | TokenKind::Match | TokenKind::Raise | TokenKind::Catch => {
                let what = format!("`{}`", self.text_of(start));
                self.later(start, &what, Some("M2"));
                ExprKind::Error
            }
            TokenKind::Comptime | TokenKind::With | TokenKind::Pipe => {
                let what = match self.peek() {
                    TokenKind::Pipe => String::from("a closure"),
                    _ => format!("`{}`", self.text_of(start)),
                };
                self.later(start, &what, Some("M4"));
                ExprKind::Error
            }
            TokenKind::Parallel | TokenKind::ParallelFor => {
                let what = format!("`{}`", self.text_of(start));
                self.later(start, &what, Some("M7"));
                ExprKind::Error
            }
            TokenKind::Invariant
            | TokenKind::Decreases
            | TokenKind::Requires
            | TokenKind::Ensures => {
                let d = Diagnostic::error(
                    "E0110",
                    start,
                    format!(
                        "`{}` starts a clause of a header, after a loop's condition or a function's signature, as in `while i < n decreases n - i {{`; it is not a statement",
                        self.text_of(start)
                    ),
                );
                self.error(d);
                ExprKind::Error
            }
            _ => {
                let d = Diagnostic::error(
                    "E0110",
                    start,
                    format!("expected an expression, found {}", self.found()),
                );
                self.error(d);
                ExprKind::Error
            }
        };
        let span = self.span_from(start);
        self.tree.push_expr(kind, span)
    }

    fn unary(&mut self, op: UnaryOp, operand_bp: u8) -> ExprKind {
        self.bump();
        let operand = self.binary(operand_bp);
        ExprKind::Unary { op, operand }
    }

    fn if_expr(&mut self) -> ExprId {
        let start = self.span();
        self.bump();
        let cond = self.cond();
        let then = if self.recovering {
            self.tree.push_block(Block {
                stmts: Vec::new(),
                span: start,
            })
        } else {
            self.block()
        };
        let mut otherwise = None;
        if !self.recovering && self.eat(&TokenKind::Else) {
            otherwise = Some(if self.at(&TokenKind::If) {
                Else::If(self.if_expr())
            } else {
                Else::Block(self.block())
            });
        }
        let span = self.span_from(start);
        self.tree.push_expr(
            ExprKind::If {
                cond,
                then,
                otherwise,
            },
            span,
        )
    }

    fn call(&mut self, callee: String) -> ExprKind {
        self.bump(); // `(`
        self.nest += 1;
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.recovering {
            let before = self.pos;
            let is_mut = self.eat(&TokenKind::Mut);
            let value = self.expr();
            if is_mut && !self.recovering && !self.is_place(value) {
                let span = self.tree.expr(value).span;
                let d = Diagnostic::error(
                    "E0114",
                    span,
                    format!(
                        "after `mut`, an argument is a place, a name or an element of one, not `{}`",
                        self.text_of(span)
                    ),
                );
                self.error(d);
            }
            args.push(Arg { is_mut, value });
            if self.pos == before {
                break;
            }
            if !self.recovering && !self.eat(&TokenKind::Comma) && !self.at(&TokenKind::RParen) {
                self.expected("`,` or `)` in the arguments");
            }
        }
        if !self.recovering {
            self.bump(); // `)`
        }
        self.nest -= 1;
        ExprKind::Call { callee, args }
    }

    /// `todo("reason")`, whose argument is a string literal.
    fn todo(&mut self) -> ExprKind {
        self.bump(); // `(`
        let reason = match (self.peek().clone(), self.peek_second()) {
            (TokenKind::StrLit(s), TokenKind::RParen) => s,
            _ => {
                let d = Diagnostic::error(
                    "E0111",
                    self.span(),
                    format!(
                        "`todo` takes its reason as a string, as in `todo(\"why\")`; found {}",
                        self.found()
                    ),
                );
                self.error(d);
                return ExprKind::Error;
            }
        };
        self.bump();
        self.bump();
        ExprKind::Todo(reason)
    }

    /// `[a, b, c]` or `[value; count]`.
    fn array(&mut self) -> ExprKind {
        self.bump(); // `[`
        self.nest += 1;
        let mut items = Vec::new();
        let mut kind = None;
        if !self.at(&TokenKind::RBracket) {
            let first = self.expr();
            if !self.recovering && self.eat(&TokenKind::Semi) {
                kind = Some(match self.peek().clone() {
                    TokenKind::IntLit(count, _) => {
                        self.bump();
                        ExprKind::Repeat {
                            value: first,
                            count,
                        }
                    }
                    _ => {
                        let d = Diagnostic::error(
                            "E0117",
                            self.span(),
                            format!(
                                "the count of `[value; count]` is an integer literal, not {}",
                                self.found()
                            ),
                        );
                        self.error(d);
                        ExprKind::Error
                    }
                });
            } else {
                items.push(first);
                while !self.recovering && self.eat(&TokenKind::Comma) {
                    if self.at(&TokenKind::RBracket) {
                        break;
                    }
                    items.push(self.expr());
                }
            }
        }
        if !self.recovering {
            self.expect(&TokenKind::RBracket, "`,` or `]` in the array");
        }
        self.nest -= 1;
        kind.unwrap_or(ExprKind::Array(items))
    }
}

/// The binding power `not` parses its operand at: below the comparisons, above `and`, so
/// `not a == b` is `not (a == b)` and `not a and b` is `(not a) and b`.
const NOT_OPERAND_BP: u8 = 3;

/// The binding power `-` and `~` parse their operand at: above every binary operator, so
/// `-a * b` is `(-a) * b`; indexing still binds tighter, so `-a[0]` is `-(a[0])`.
const UNARY_OPERAND_BP: u8 = 11;

/// A binary operator and its left binding power; each groups to the left.
fn infix(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    Some(match kind {
        TokenKind::Or => (BinaryOp::Or, 1),
        TokenKind::And => (BinaryOp::And, 2),
        TokenKind::EqEq => (BinaryOp::Eq, 4),
        TokenKind::NotEq => (BinaryOp::Ne, 4),
        TokenKind::Lt => (BinaryOp::Lt, 4),
        TokenKind::Le => (BinaryOp::Le, 4),
        TokenKind::Gt => (BinaryOp::Gt, 4),
        TokenKind::Ge => (BinaryOp::Ge, 4),
        TokenKind::Pipe => (BinaryOp::BitOr, 5),
        TokenKind::Caret => (BinaryOp::BitXor, 6),
        TokenKind::Amp => (BinaryOp::BitAnd, 7),
        TokenKind::Shl => (BinaryOp::Shl, 8),
        TokenKind::Shr => (BinaryOp::Shr, 8),
        TokenKind::Plus => (BinaryOp::Add, 9),
        TokenKind::Minus => (BinaryOp::Sub, 9),
        TokenKind::Star => (BinaryOp::Mul, 10),
        TokenKind::Slash => (BinaryOp::Div, 10),
        TokenKind::Percent => (BinaryOp::Rem, 10),
        _ => return None,
    })
}

fn is_comparison(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

fn assign_op(kind: &TokenKind) -> Option<AssignOp> {
    Some(match kind {
        TokenKind::Eq => AssignOp::Set,
        TokenKind::PlusEq => AssignOp::Compound(BinaryOp::Add),
        TokenKind::MinusEq => AssignOp::Compound(BinaryOp::Sub),
        TokenKind::StarEq => AssignOp::Compound(BinaryOp::Mul),
        TokenKind::SlashEq => AssignOp::Compound(BinaryOp::Div),
        TokenKind::PercentEq => AssignOp::Compound(BinaryOp::Rem),
        TokenKind::AmpEq => AssignOp::Compound(BinaryOp::BitAnd),
        TokenKind::PipeEq => AssignOp::Compound(BinaryOp::BitOr),
        TokenKind::CaretEq => AssignOp::Compound(BinaryOp::BitXor),
        TokenKind::ShlEq => AssignOp::Compound(BinaryOp::Shl),
        TokenKind::ShrEq => AssignOp::Compound(BinaryOp::Shr),
        _ => return None,
    })
}

/// The suffix of an integer literal as written: `u8` for `255_u8` and `0xFF_u8`.
fn int_suffix(raw: &str) -> Option<String> {
    let hex = raw.starts_with("0x");
    let bytes = raw.as_bytes();
    (1..bytes.len())
        .find(|&i| {
            bytes[i - 1] == b'_'
                && bytes[i].is_ascii_alphabetic()
                && !(hex && bytes[i].is_ascii_hexdigit())
        })
        .map(|i| raw[i..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::apply_fix;
    use crate::lex;

    /// Parses `src`, which holds one function and nothing else.
    fn parse(src: &str) -> (FnSyntax, Vec<Diagnostic>) {
        let (tokens, errors) = lex(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();
        parse_function(src, &tokens)
    }

    /// The canonical form of an expression body.
    fn expr(src: &str) -> String {
        let (f, errors) = parse(&format!("fn f() = {}", src));
        assert!(errors.is_empty(), "{}: {:?}", src, errors);
        match f.body {
            Some(Body::Expr(e)) => f.tree.render_expr(e),
            other => panic!("{}: {:?}", src, other),
        }
    }

    /// The canonical form of a block body holding `src`.
    fn block(src: &str) -> String {
        let (f, errors) = parse(&format!("fn f() {{\n{}\n}}", src));
        assert!(errors.is_empty(), "{}: {:?}", src, errors);
        match f.body {
            Some(Body::Block(b)) => f.tree.render_block(b),
            other => panic!("{}: {:?}", src, other),
        }
    }

    /// The codes and messages of a block body holding `src`.
    fn errors(src: &str) -> Vec<(&'static str, String)> {
        let (_, errors) = parse(&format!("fn f() {{\n{}\n}}", src));
        errors.into_iter().map(|d| (d.code, d.message)).collect()
    }

    fn codes(src: &str) -> Vec<&'static str> {
        errors(src).into_iter().map(|e| e.0).collect()
    }

    #[test]
    fn test_operators_bind_and_group_as_the_grammar_says() {
        let cases = [
            ("a or b and c", "(or a (and b c))"),
            ("a and b or c and d", "(or (and a b) (and c d))"),
            ("not a == b", "(not (== a b))"),
            ("not a and b", "(and (not a) b)"),
            ("not not a", "(not (not a))"),
            ("a == b or c", "(or (== a b) c)"),
            ("x & mask == 0", "(== (& x mask) 0)"),
            ("a | b == c", "(== (| a b) c)"),
            ("a | b ^ c & d", "(| a (^ b (& c d)))"),
            ("a ^ b | c", "(| (^ a b) c)"),
            ("a & b ^ c", "(^ (& a b) c)"),
            ("a << 1 + b", "(<< a (+ 1 b))"),
            ("a + b << c", "(<< (+ a b) c)"),
            ("a >> 2 < b", "(< (>> a 2) b)"),
            ("a & b << c", "(& a (<< b c))"),
            ("a + b * c - d", "(- (+ a (* b c)) d)"),
            ("a - b - c", "(- (- a b) c)"),
            ("a / b % c", "(% (/ a b) c)"),
            ("a * b + c", "(+ (* a b) c)"),
            ("-a * b", "(* (- a) b)"),
            ("-a[0]", "(- (index a 0))"),
            ("~x & y", "(& (~ x) y)"),
            ("- -x", "(- (- x))"),
            ("(a + b) * c", "(* (+ a b) c)"),
            (
                "a != b and c <= d and e >= f and g > h",
                "(and (and (and (!= a b) (<= c d)) (>= e f)) (> g h))",
            ),
            ("255_u8 + 0xFF_u8", "(+ 255_u8 255_u8)"),
            ("true and false", "(and true false)"),
            ("f(a, mut b, c[1])", "(call f a mut b (index c 1))"),
            ("f()", "(call f)"),
            ("f(a,)", "(call f a)"),
            ("a.len - 1", "(- (. a len) 1)"),
            ("a[i][j]", "(index (index a i) j)"),
            ("[1, 2, 3]", "[1 2 3]"),
            ("[1, 2,]", "[1 2]"),
            ("[]", "[]"),
            ("[0; 4]", "[0; 4]"),
            ("[[0; 2]; 3]", "[[0; 2]; 3]"),
            ("todo(\"later\")", "(todo \"later\")"),
            ("if c { 1 } else { 2 }", "(if c {1} {2})"),
            (
                "if a { 1 } else if b { 2 } else { 3 }",
                "(if a {1} (if b {2} {3}))",
            ),
            ("f(if c { 1 } else { 2 })", "(call f (if c {1} {2}))"),
        ];
        for (src, want) in cases {
            assert_eq!(expr(src), want, "{}", src);
        }
    }

    #[test]
    fn test_statements_of_m1_parse() {
        assert_eq!(
            block("val x = 1\nvar y: u8[0..10] = 2\ny += x\ny = y * 2\na[i] = 3\na[i][j] -= 1"),
            "{(val x 1); (var y u8[0..10] 2); (+= y x); (= y (* y 2)); (= (index a i) 3); (-= (index (index a i) j) 1)}"
        );
        assert_eq!(
            block("x *= 1\nx /= 2\nx %= 3\nx &= 4\nx |= 5\nx ^= 6\nx <<= 7\nx >>= 8"),
            "{(*= x 1); (/= x 2); (%= x 3); (&= x 4); (|= x 5); (^= x 6); (<<= x 7); (>>= x 8)}"
        );
        assert_eq!(
            block("while lo < hi invariant lo <= hi decreases hi - lo {\n  lo += 1\n  continue\n}"),
            "{(while (< lo hi) (invariant (<= lo hi)) (decreases (- hi lo)) {(+= lo 1); continue})}"
        );
        // A loop's clauses may each take a line, and so may `else` and a block's brace.
        assert_eq!(
            block("while i < n\n  invariant i <= n\n{\n  i += 1\n  break\n}"),
            "{(while (< i n) (invariant (<= i n)) {(+= i 1); break})}"
        );
        assert_eq!(
            block("if c\n{\n  x = 1\n}\nelse\n{\n  x = 2\n}"),
            "{(if c {(= x 1)} {(= x 2)})}"
        );
        assert_eq!(
            block("guard x > 0 else {\n  return 0\n}\nreturn"),
            "{(guard (> x 0) {(return 0)}); (return)}"
        );
        assert_eq!(block("if c { return }"), "{(if c {(return)})}");
        assert_eq!(
            block("val a: [4]bool = [true; 4]\nval b: i32[-5..=n] = 0\nval c: usize[0..a.len] = 1"),
            "{(val a [4]bool [true; 4]); (val b i32[-5..=n] 0); (val c usize[0..a.len] 1)}"
        );
        assert_eq!(block(""), "{}");
    }

    #[test]
    fn test_a_newline_ends_a_statement_where_the_grammar_says() {
        // After a token that cannot end a statement, the statement goes on.
        assert_eq!(block("val x = a +\n  b"), "{(val x (+ a b))}");
        assert_eq!(block("val x = not\n  b"), "{(val x (not b))}");
        assert_eq!(block("val x =\n  b"), "{(val x b)}");
        assert_eq!(block("x +=\n  b"), "{(+= x b)}");
        // Inside parentheses and brackets, a newline is whitespace.
        assert_eq!(block("val x = f(a,\n  b)"), "{(val x (call f a b))}");
        assert_eq!(block("val y = [1,\n  2]"), "{(val y [1 2])}");
        assert_eq!(block("val z = (a\n  + b)"), "{(val z (+ a b))}");
        assert_eq!(block("val w = a[i\n  + 1]"), "{(val w (index a (+ i 1)))}");
        // Elsewhere, a newline before an operator ends the statement.
        assert_eq!(block("val x = a\n- b"), "{(val x a); (- b)}");
        assert_eq!(block("val z = a\n[0]"), "{(val z a); [0]}");
        // A bracket closed on its line leaves no newline swallowed after it.
        for opened in ["a[0]", "f(1)", "(1)", "[1]"] {
            assert_eq!(
                block(&format!("val w = {}\n- b", opened)),
                format!("{{(val w {}); (- b)}}", expr(opened)),
                "{}",
                opened
            );
        }
        assert_eq!(
            block("val y = if c { 1 } else { 2 }\n-x"),
            "{(val y (if c {1} {2})); (- x)}"
        );
        assert_eq!(codes("x\n= 1"), vec!["E0110"]);
        assert_eq!(codes("val x = a +\n)"), vec!["E0110"]);
        assert_eq!(codes("val q = a\n.len"), vec!["E0110"]);
    }

    #[test]
    fn test_contracts_and_parameters_of_a_header() {
        let (f, errors) = parse(
            "fn clamp(mut a: u8, b: [4]bool) -> u8[0..=255]\n  requires a > 0\n  ensures result >= a\n  decreases a\n= a",
        );
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(f.name, "clamp");
        let params: Vec<(bool, &str, String)> = f
            .params
            .iter()
            .map(|p| (p.is_mut, p.name.as_str(), f.tree.render_type(p.ty)))
            .collect();
        assert_eq!(
            params,
            vec![
                (true, "a", String::from("u8")),
                (false, "b", String::from("[4]bool"))
            ]
        );
        assert_eq!(f.tree.render_type(f.result.unwrap()), "u8[0..=255]");
        let contracts: Vec<(ContractKind, String)> = f
            .contracts
            .iter()
            .map(|c| (c.kind, f.tree.render_expr(c.expr)))
            .collect();
        assert_eq!(
            contracts,
            vec![
                (ContractKind::Requires, String::from("(> a 0)")),
                (ContractKind::Ensures, String::from("(>= result a)")),
                (ContractKind::Decreases, String::from("a")),
            ]
        );
        // A function without a body has none, and a missing body is reported.
        let (f, errors) = parse("fn g(x: i64) -> bool");
        assert!(errors.is_empty() && f.body.is_none());
        // A contract ends at its line: an operator starting the next one is not part of it.
        let (_, errors) = parse("fn g(a: u8) -> u8\n  requires a\n  > 0\n= a");
        assert_eq!(
            errors[0].message,
            "expected `{` or `=` and the function's body, found `>`"
        );
        let (_, errors) = parse("fn g(x: i64) -> bool junk");
        assert_eq!(
            errors[0].message,
            "expected `{` or `=` and the function's body, found `junk`"
        );
    }

    #[test]
    fn test_spans_cover_what_each_node_was_parsed_from() {
        let src = "fn f(x: u8) = x + 2 * x";
        let (f, _) = parse(src);
        let Some(Body::Expr(e)) = f.body else {
            panic!("no body")
        };
        let span = f.tree.expr(e).span;
        assert_eq!(&src[span.start..span.end], "x + 2 * x");
        let ty = f.tree.ty(f.params[0].ty).span;
        assert_eq!(&src[ty.start..ty.end], "u8");
        let src = "fn f() {\n  val total: u8[0..9] = 1\n}";
        let (f, _) = parse(src);
        let stmt = &f.tree.stmts[0];
        assert_eq!(
            &src[stmt.span.start..stmt.span.end],
            "val total: u8[0..9] = 1"
        );
        let StmtKind::Binding { name_span, ty, .. } = &stmt.kind else {
            panic!("not a binding")
        };
        assert_eq!(&src[name_span.start..name_span.end], "total");
        let ty = f.tree.ty(ty.unwrap()).span;
        assert_eq!(&src[ty.start..ty.end], "u8[0..9]");
        let b = f.tree.blocks.last().unwrap().span;
        assert_eq!(&src[b.start..b.end], "{\n  val total: u8[0..9] = 1\n}");
    }

    #[test]
    fn test_parse_errors_have_their_codes_and_recovery_reports_one() {
        assert_eq!(
            errors("val x = )"),
            vec![("E0110", String::from("expected an expression, found `)`"))]
        );
        assert_eq!(
            errors("val x 1"),
            vec![(
                "E0111",
                String::from("expected `=` and the binding's value, found `1`")
            )]
        );
        assert_eq!(
            errors("f(a b)"),
            vec![(
                "E0111",
                String::from("expected `,` or `)` in the arguments, found `b`")
            )]
        );
        assert_eq!(codes("guard x {\n  return\n}"), vec!["E0111"]);
        assert_eq!(codes("val a = [1, 2\nval b = 3"), vec!["E0111"]);
        assert_eq!(codes("val a = (1\nval b = 3"), vec!["E0111"]);
        assert_eq!(codes("val a = b[1\n}"), vec!["E0111"]);
        assert_eq!(
            errors("todo(5)"),
            vec![(
                "E0111",
                String::from(
                    "`todo` takes its reason as a string, as in `todo(\"why\")`; found `5`"
                )
            )]
        );
        assert_eq!(codes("if c 1"), vec!["E0111"]);
        assert_eq!(codes("val match = 1"), vec!["E0101"]);
        // A loop's clause belongs in its header (`grammar.md`, rejected alternatives).
        for clause in ["invariant", "decreases", "requires", "ensures"] {
            assert_eq!(
                errors(&format!("while n > 0 {{\n  {} n\n}}", clause)),
                vec![(
                    "E0110",
                    format!(
                        "`{}` starts a clause of a header, after a loop's condition or a function's signature, as in `while i < n decreases n - i {{`; it is not a statement",
                        clause
                    )
                )]
            );
        }
        assert_eq!(codes("val x: = 1"), vec!["E0116"]);
        // One mistake, one diagnostic: the next statement parses.
        let (f, errs) = parse("fn f() {\n  val x = (1 + )\n  val y = 2\n}");
        assert_eq!(errs.len(), 1);
        assert!(f
            .tree
            .stmts
            .iter()
            .any(|s| matches!(&s.kind, StmtKind::Binding { name, .. } if name == "y")));
        // A construct that spans lines is skipped whole.
        let (f, errs) = parse("fn f() {\n  val x = match y {\n    1 => 2\n  }\n  val z = 3\n}");
        assert_eq!(errs.len(), 1);
        assert!(f
            .tree
            .stmts
            .iter()
            .any(|s| matches!(&s.kind, StmtKind::Binding { name, .. } if name == "z")));
        // A header error leaves the body to be parsed.
        let (f, errs) = parse("fn f(a u8) = 1 + ");
        assert_eq!(
            errs.iter()
                .map(|d| (d.code, d.message.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("E0111", "expected `:` and the parameter's type, found `u8`"),
                (
                    "E0110",
                    "expected an expression, found the end of the function"
                ),
            ]
        );
        let (_, errs) = parse("fn f() = todo(");
        assert_eq!(errs[0].code, "E0111");
        assert!(f.body.is_some());
        let (_, errs) = parse("fn f() = a b");
        assert_eq!(
            errs[0].message,
            "expected the end of the expression body, found `b`"
        );
        // Recovery that reaches the end of the tokens stops there.
        let (_, errs) = parse("fn f() {\n  val x = (1 +");
        assert_eq!(
            errs.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec!["E0110", "E0111"]
        );
        // A block the tokens never close.
        let (_, errs) = parse("fn f() {\n  val x = 1");
        assert_eq!(errs[0].message, "this block has no closing `}`");
    }

    #[test]
    fn test_statements_share_no_line_and_semicolons_get_a_fix() {
        let src = "fn f() {\n  x = 1 y = 2\n}";
        let (_, errs) = parse(src);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].code, "E0112");
        assert_eq!(
            apply_fix(src, &errs[0].fixes[0]).unwrap(),
            "fn f() {\n  x = 1\ny = 2\n}"
        );
        let src = "fn f() {\n  x = 1;\n}";
        let (_, errs) = parse(src);
        assert_eq!(
            errs[0].message,
            "`;` does not end a statement in Mente; a newline does"
        );
        assert_eq!(
            apply_fix(src, &errs[0].fixes[0]).unwrap(),
            "fn f() {\n  x = 1\n}"
        );
        let src = "fn f() { x = 1; }";
        let (_, errs) = parse(src);
        assert_eq!(
            apply_fix(src, &errs[0].fixes[0]).unwrap(),
            "fn f() { x = 1 }"
        );
        let src = "fn f() {\n  x = 1; y = 2\n}";
        let (f, errs) = parse(src);
        assert_eq!(errs.len(), 1);
        assert_eq!(
            apply_fix(src, &errs[0].fixes[0]).unwrap(),
            "fn f() {\n  x = 1\ny = 2\n}"
        );
        assert_eq!(f.tree.stmts.len(), 2);
    }

    #[test]
    fn test_chained_comparisons_places_and_if_operands_are_errors() {
        let src = "fn f() = a < b < c";
        let (_, errs) = parse(src);
        assert_eq!(errs[0].code, "E0113");
        assert_eq!(
            errs[0].message,
            "comparisons do not chain; write `... b and b ...`"
        );
        assert_eq!(errs[0].fixes[0].applicability, Applicability::Suggestion);
        assert_eq!(
            apply_fix(src, &errs[0].fixes[0]).unwrap(),
            "fn f() = a < b and b < c"
        );
        // Parentheses make a comparison an operand like any other.
        assert_eq!(expr("(a < b) == c"), "(== (< a b) c)");

        assert_eq!(
            errors("a + b = 1"),
            vec![(
                "E0114",
                String::from(
                    "`a + b` is not a place to assign to; a place is a name, or an element of one such as `a[i]`"
                )
            )]
        );
        assert_eq!(
            errors("f(mut a + b)"),
            vec![(
                "E0114",
                String::from(
                    "after `mut`, an argument is a place, a name or an element of one, not `a + b`"
                )
            )]
        );
        assert!(codes("f(mut a[i])").is_empty());

        for src in [
            "fn f() = 1 + if c { 2 } else { 3 }",
            "fn f() = if c { 1 } else { 2 } + 3",
        ] {
            let (_, errs) = parse(src);
            assert_eq!(errs.len(), 1, "{}", src);
            assert_eq!(errs[0].code, "E0115");
            assert_eq!(
                errs[0].fixes[0].applicability,
                Applicability::MachineApplicable
            );
            let fixed = apply_fix(src, &errs[0].fixes[0]).unwrap();
            assert!(fixed.contains("(if c {"), "{}", fixed);
            assert!(parse(&fixed).1.is_empty(), "{}", fixed);
        }
    }

    #[test]
    fn test_types_m1_lacks_and_malformed_ones_are_reported() {
        let cases = [
            (
                "fn f(x: int) = x",
                "E0116",
                "`int` is not a type of M1, whose types are the integer types, `bool` and arrays `[N]T`; structs and enums come in M2",
            ),
            (
                "fn f(x: [n]u8) = x",
                "E0116",
                "an array's length is an integer literal, as in `[4]u8`",
            ),
            (
                "fn f(x: u8[0..]) = x",
                "E0116",
                "a refinement's bound is an integer literal, a name, or `name.len`, not `]`",
            ),
            (
                "fn f(x: u8[-n..4]) = x",
                "E0116",
                "a refinement's bound is an integer literal, a name, or `name.len`, not `n`",
            ),
            (
                "fn f(x: u8[0 4]) = x",
                "E0111",
                "expected `..` or `..=` between the refinement's bounds, found `4`",
            ),
            (
                "fn f(x: u8[0..4) = x",
                "E0111",
                "expected `]` to close the refinement, found `)`",
            ),
            (
                "fn f(x: [4 u8) = x",
                "E0111",
                "expected `]` after the array's length, found `u8`",
            ),
            ("fn f(x: )", "E0116", "expected a type, found `)`"),
            (
                "fn f(x: u8[0..n.x]) = x",
                "E0111",
                "expected `]` to close the refinement, found `.`",
            ),
        ];
        for (src, code, message) in cases {
            let (_, errs) = parse(src);
            assert_eq!(
                errs.into_iter()
                    .map(|d| (d.code, d.message))
                    .collect::<Vec<_>>(),
                vec![(code, String::from(message))],
                "{}",
                src
            );
        }
        assert_eq!(
            errors("val a = [0; n]"),
            vec![(
                "E0117",
                String::from("the count of `[value; count]` is an integer literal, not `n`")
            )]
        );
    }

    #[test]
    fn test_constructs_of_later_milestones_name_their_milestone() {
        let in_body = [
            (
                "val s = \"text\"",
                "a string value comes in M2; M1 does not have it",
            ),
            (
                "val x = 1.5",
                "a floating-point or fixed-point value comes in M5; M1 does not have it",
            ),
            (
                "val m = match x {\n  1 => 2\n}",
                "`match` comes in M2; M1 does not have it",
            ),
            ("raise E.Bad", "`raise` comes in M2; M1 does not have it"),
            ("val n = none", "`none` comes in M2; M1 does not have it"),
            ("val c = catch", "`catch` comes in M2; M1 does not have it"),
            (
                "val k = comptime f()",
                "`comptime` comes in M4; M1 does not have it",
            ),
            (
                "with Arena.scoped() {\n}",
                "`with` comes in M4; M1 does not have it",
            ),
            (
                "val g = |x| x",
                "a closure comes in M4; M1 does not have it",
            ),
            (
                "parallel (a(), b())",
                "`parallel` comes in M7; M1 does not have it",
            ),
            (
                "parallel_for x in xs {\n}",
                "`parallel_for` comes in M7; M1 does not have it",
            ),
            (
                "for i in 0..4 {\n}",
                "a `for` loop is in no milestone yet; M1 does not have it",
            ),
            (
                "val l = xs.count()",
                "a method call comes in M2; M1 does not have it",
            ),
            (
                "val p = point.x",
                "a field other than `.len` comes in M2; M1 does not have it",
            ),
            (
                "val r = f()?",
                "error propagation (`?` and `!`) comes in M2; M1 does not have it",
            ),
            (
                "val r = f()!",
                "error propagation (`?` and `!`) comes in M2; M1 does not have it",
            ),
            (
                "val o = a ?? b",
                "an optional value's operator (`??` or `?.`) comes in M2; M1 does not have it",
            ),
            (
                "val o = a?.b",
                "an optional value's operator (`??` or `?.`) comes in M2; M1 does not have it",
            ),
            (
                "val y = x |> f()",
                "the pipe `|>` is in no milestone yet; M1 does not have it",
            ),
            (
                "val r = 0..4",
                "a range in an expression is in no milestone yet; M1 does not have it",
            ),
            (
                "val r = 0..=4",
                "a range in an expression is in no milestone yet; M1 does not have it",
            ),
            (
                "pool[mut h] = 1",
                "a `mut` projection `p[mut h]` comes in M3; M1 does not have it",
            ),
        ];
        for (src, message) in in_body {
            assert_eq!(
                errors(src),
                vec![("E0109", String::from(message))],
                "{}",
                src
            );
        }
        let in_header = [
            (
                "fn f<T>(x: T) = x",
                "a type parameter list comes in M4; M1 does not have it",
            ),
            (
                "fn f(own x: u8) = x",
                "an `own` parameter comes in M2; M1 does not have it",
            ),
            (
                "fn f(in a: u8) = a",
                "an `in` parameter comes in M2; M1 does not have it",
            ),
            (
                "fn f() -> u8 ! E = 1",
                "an error type `T ! E` comes in M2; M1 does not have it",
            ),
            (
                "fn f(d: [4]u8) -> u8 from d = 1",
                "a `from` clause comes in M2; M1 does not have it",
            ),
            (
                "fn f() -> u8 with IO = 1",
                "an effect clause `with` comes in M4; M1 does not have it",
            ),
            (
                "fn f(x: []u8) = 1",
                "a slice type `[]T` comes in M2; M1 does not have it",
            ),
            (
                "fn f(x: ?u8) = 1",
                "an optional type `?T` comes in M2; M1 does not have it",
            ),
            (
                "fn f(x: str) = 1",
                "a string or list type (`str`) comes in M2; M1 does not have it",
            ),
            (
                "fn f(x: f64) = 1",
                "a floating-point, decimal or saturating type (`f64`) comes in M5; M1 does not have it",
            ),
            (
                "fn f(x: Pool) = 1",
                "an arena, pool or layout type (`Pool`) comes in M3; M1 does not have it",
            ),
        ];
        for (src, message) in in_header {
            let (f, errs) = parse(src);
            assert_eq!(
                errs.into_iter()
                    .map(|d| (d.code, d.message))
                    .collect::<Vec<_>>(),
                vec![("E0109", String::from(message))],
                "{}",
                src
            );
            assert!(
                f.body.is_some(),
                "{}: the body is parsed after the header",
                src
            );
        }
    }

    #[test]
    fn test_the_parser_never_moves_past_the_end() {
        let src = "fn f()";
        let (tokens, _) = lex(src);
        let mut p = Parser {
            tokens,
            pos: 0,
            text: src.to_string(),
            base: 0,
            tree: Tree::default(),
            errors: Vec::new(),
            recovering: false,
            nest: 0,
        };
        for _ in 0..6 {
            p.bump();
        }
        assert_eq!(p.pos, p.tokens.len() - 1);
        assert_eq!(p.peek(), &TokenKind::Eof);
        assert_eq!(p.peek_second(), &TokenKind::Eof);
    }

    #[test]
    fn test_integer_suffixes_are_read_from_the_literal() {
        assert_eq!(int_suffix("255_u8"), Some(String::from("u8")));
        assert_eq!(int_suffix("0xFF_u8"), Some(String::from("u8")));
        assert_eq!(int_suffix("0b1_i32"), Some(String::from("i32")));
        assert_eq!(int_suffix("0xab_cd"), None);
        assert_eq!(int_suffix("1_000"), None);
        assert_eq!(int_suffix("7"), None);
    }
}

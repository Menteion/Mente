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
    Arg, Arm, ArmBody, AssignOp, BinaryOp, Block, BlockId, Body, Bound, Contract, ContractKind,
    Else, EnumSyntax, ExprId, ExprKind, FieldInit, FieldSyntax, FnSyntax, Param, PatBinding,
    Pattern, PatternKind, Payload, Refinement, StmtId, StmtKind, StructSyntax, Tree, TypeId,
    TypeKind, UnaryOp, VariantSyntax,
};
use crate::{Span, Token, TokenKind};

/// The integer types of M1 (`grammar.md`, item 5).
pub const INT_TYPES: [&str; 11] = [
    "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "usize",
];

/// A type the language names but a later milestone brings: the milestone, and what it is.
pub fn later_type(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "str" | "String" | "List" => Some(("M2", "a string or list type")),
        "f32" | "f64" | "dec" | "sat" | "wrap" => {
            Some(("M5", "a floating-point, decimal or saturating type"))
        }
        "Arena" | "Pool" | "Handle" | "Local" | "Shared" | "SoA" | "Tiled" => {
            Some(("M3", "an arena, pool or layout type"))
        }
        _ => None,
    }
}

/// Whether `name` names a type of the language, now or in a later milestone: an integer type,
/// `bool`, or a [`later_type`]. No declaration of a module takes such a name.
pub fn language_type(name: &str) -> bool {
    INT_TYPES.contains(&name) || name == "bool" || later_type(name).is_some()
}

/// Parses one function from its tokens, `fn` to the end of its body, which `src` holds at the
/// tokens' spans.
pub fn parse_function(src: &str, tokens: &[Token]) -> (FnSyntax, Vec<Diagnostic>) {
    let mut p = Parser::new(src, tokens);
    let syntax = p.function();
    // A tree too high for the phases that recurse over it, such as a long chain of `+`,
    // which the parser reads in a loop: measured after another error too, since a phase that
    // reads a tree with errors, as `mn dump --tree` does, recurses over it all the same.
    if !p.errors.iter().any(|e| e.code == "E0127") {
        if let Some(span) = syntax.tree.too_deep(NESTING_LIMIT) {
            p.errors.push(too_deep(span));
        }
    }
    (syntax, p.errors)
}

/// Parses one struct declaration from its tokens, `struct` to its closing `}`, which `src`
/// holds at the tokens' spans (`grammar.md`, item 10): one field per line, each a name and a
/// type. Type parameters, an `invariant` block and a field's default are constructs of later
/// milestones (`E0109`); a comma after a field is another language's separator (`E0128`), whose
/// fix removes it.
pub fn parse_struct(src: &str, tokens: &[Token]) -> (StructSyntax, Vec<Diagnostic>) {
    let mut p = Parser::new(src, tokens);
    let syntax = p.struct_decl();
    (syntax, p.errors)
}

/// Parses one enum declaration from its tokens, `enum` to its closing `}`, which `src` holds at
/// the tokens' spans (`grammar.md`, item 10): one variant per line, each a name and, in
/// parentheses, its payload's named fields. Type parameters are a construct of a later milestone
/// (`E0109`); a comma after a variant is another language's separator (`E0128`), and a payload of
/// unnamed types another language's (`E0130`), each with a fix.
pub fn parse_enum(src: &str, tokens: &[Token]) -> (EnumSyntax, Vec<Diagnostic>) {
    let mut p = Parser::new(src, tokens);
    let syntax = p.enum_decl();
    (syntax, p.errors)
}

/// How deep expressions, blocks and types may nest in a function: the parser and every phase
/// after it recurse over them, and none may run out of stack.
pub const NESTING_LIMIT: u32 = 256;

fn too_deep(span: Span) -> Diagnostic {
    Diagnostic::error(
        "E0127",
        span,
        format!(
            "this code nests deeper than {} levels of expressions, blocks and types; give a nested part a name with `val`, or a function of its own",
            NESTING_LIMIT
        ),
    )
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
    /// How deep the parser has descended into expressions, blocks and types.
    depth: u32,
    /// `nest` inside the brackets of the index being parsed, where a range makes a view.
    index_nest: Option<u32>,
    /// `nest` where the condition or clause being parsed stands, where a `{` after a name opens
    /// the block that follows, never another language's struct literal.
    cond_nest: Option<u32>,
}

impl Parser {
    fn new(src: &str, tokens: &[Token]) -> Parser {
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
        Parser {
            text: src[base..last.end.max(base)].to_string(),
            base,
            tokens,
            pos: 0,
            tree: Tree::default(),
            errors: Vec::new(),
            recovering: false,
            nest: 0,
            depth: 0,
            index_nest: None,
            cond_nest: None,
        }
    }

    // ---------------------------------------------------------------- tokens

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    /// The token after the next one, or the end.
    fn second(&self) -> &Token {
        &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)]
    }

    fn peek_second(&self) -> &TokenKind {
        &self.second().kind
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

    /// Descends one level, or reports the function as nesting too deep (`E0127`) and skips to
    /// its end, so that the parser itself never recurses past the limit.
    fn deeper(&mut self) -> bool {
        if self.depth >= NESTING_LIMIT {
            let d = too_deep(self.span());
            self.error(d);
            self.pos = self.tokens.len() - 1;
            return false;
        }
        self.depth += 1;
        true
    }

    /// Reports `d`, unless the parser is already recovering from an error.
    fn error(&mut self, d: Diagnostic) {
        // After `E0127` the rest of the function is skipped, and nothing more is said of it.
        if !self.recovering && !self.errors.iter().any(|e| e.code == "E0127") {
            self.errors.push(d);
        }
        self.recovering = true;
    }

    /// Reports `d` and goes on: for another language's idiom whose meaning is plain, so the
    /// rest of the statement is still checked. An idiom is read only outside recovery, since
    /// an error stops its statement's parse before any idiom after it.
    fn note(&mut self, d: Diagnostic) {
        debug_assert!(!self.recovering, "an idiom noted while recovering");
        if !self.errors.iter().any(|e| e.code == "E0127") {
            self.errors.push(d);
        }
    }

    /// Whether the next two tokens touch, as the two characters of `&&` do.
    fn next_two_touch(&self) -> bool {
        self.span().end == self.second().span.start
    }

    /// The index of the `)` that closes the `(` at the next token, or of the end.
    fn closing_paren(&self) -> usize {
        let mut depth = 0i32;
        for (i, tok) in self.tokens.iter().enumerate().skip(self.pos) {
            match tok.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen if depth == 1 => return i,
                TokenKind::RParen => depth -= 1,
                _ => {}
            }
        }
        self.tokens.len() - 1
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
        let message = crate::diagnostics::later_message(what, milestone);
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
            let outer = self.cond_nest.replace(self.nest);
            let expr = self.expr();
            self.cond_nest = outer;
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

    // ---------------------------------------------------------------- structs

    fn struct_decl(&mut self) -> StructSyntax {
        self.bump(); // `struct`
        let (name, name_span) = self.name("a struct's name");
        let mut fields = Vec::new();
        if !self.recovering && self.at(&TokenKind::Lt) {
            let span = self.span();
            self.later(span, "a struct with type parameters", Some("M4"));
            while !matches!(self.peek(), TokenKind::LBrace | TokenKind::Eof) {
                self.bump();
            }
            self.recovering = false;
        }
        if !self.recovering && self.expect(&TokenKind::LBrace, "`{` and the struct's fields") {
            loop {
                match self.peek() {
                    TokenKind::RBrace | TokenKind::Eof => break,
                    TokenKind::DocComment(_) => {
                        self.bump();
                        continue;
                    }
                    _ => {}
                }
                let before = self.pos;
                self.field(&mut fields);
                if self.recovering {
                    self.skip_statement();
                } else {
                    self.statement_end("a field");
                }
                if self.pos == before {
                    self.bump();
                }
            }
            if fields.is_empty() && self.errors.is_empty() {
                let d = Diagnostic::error(
                    "E0111",
                    name_span,
                    format!(
                        "`struct {}` has no field; a struct has at least one, as in `x: i32` on a line of its own",
                        name
                    ),
                );
                self.error(d);
            }
        }
        if !self.errors.iter().any(|e| e.code == "E0127") {
            if let Some(span) = self.tree.too_deep(NESTING_LIMIT) {
                self.errors.push(too_deep(span));
            }
        }
        StructSyntax {
            name,
            name_span,
            fields,
            tree: std::mem::take(&mut self.tree),
        }
    }

    /// One field of a struct, `{ attribute } [ "pub" ] name ":" type`, or an `invariant` block,
    /// which a later milestone brings.
    fn field(&mut self, fields: &mut Vec<FieldSyntax>) {
        if self.at(&TokenKind::Invariant) {
            let span = self.span();
            self.later(
                span,
                "an invariant relating a struct's fields",
                Some("a later milestone"),
            );
            self.bump();
            if self.at(&TokenKind::LBrace) {
                let mut depth = 0i32;
                loop {
                    match self.peek() {
                        TokenKind::Eof => break,
                        TokenKind::LBrace => depth += 1,
                        TokenKind::RBrace => depth -= 1,
                        _ => {}
                    }
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                }
            }
            self.recovering = false;
            return;
        }
        // An attribute of a field is a layout's (`morphic-layouts.md`), which a struct without
        // `@layout` does not read.
        self.skip_attributes();
        if self.recovering {
            return;
        }
        let is_pub = self.eat(&TokenKind::Pub);
        let (name, name_span) = self.name("a field's name");
        if self.recovering || !self.expect(&TokenKind::Colon, "`:` and the field's type") {
            return;
        }
        let ty = self.ty();
        if self.recovering {
            return;
        }
        fields.push(FieldSyntax {
            is_pub,
            name,
            name_span,
            ty,
        });
        if self.at(&TokenKind::Eq) && !self.newline_before() {
            let span = self.span();
            self.later(span, "a field's default value", None);
            return;
        }
        self.comma_after("a field");
    }

    /// A `,` at the end of a field, a variant or an arm, which ends at the end of its line
    /// (`E0128`), whose fix removes it.
    fn comma_after(&mut self, what: &str) {
        if !self.at(&TokenKind::Comma) || self.newline_before() {
            return;
        }
        let comma = self.bump().span;
        let d = Diagnostic::error(
            "E0128",
            comma,
            format!(
                "{} ends at the end of its line, so Mente writes no `,` after one",
                what
            ),
        )
        .with_fix(
            Applicability::MachineApplicable,
            "remove the `,`",
            comma,
            "",
        );
        self.note(d);
    }

    // ---------------------------------------------------------------- enums

    fn enum_decl(&mut self) -> EnumSyntax {
        self.bump(); // `enum`
        let (name, name_span) = self.name("an enum's name");
        let mut variants = Vec::new();
        if !self.recovering && self.at(&TokenKind::Lt) {
            let span = self.span();
            self.later(span, "an enum with type parameters", Some("M4"));
            while !matches!(self.peek(), TokenKind::LBrace | TokenKind::Eof) {
                self.bump();
            }
            self.recovering = false;
        }
        if !self.recovering && self.expect(&TokenKind::LBrace, "`{` and the enum's variants") {
            loop {
                match self.peek() {
                    TokenKind::RBrace | TokenKind::Eof => break,
                    TokenKind::DocComment(_) => {
                        self.bump();
                        continue;
                    }
                    _ => {}
                }
                let before = self.pos;
                self.variant(&mut variants);
                if self.recovering {
                    self.skip_statement();
                } else {
                    self.statement_end("a variant");
                }
                if self.pos == before {
                    self.bump();
                }
            }
            if variants.is_empty() && self.errors.is_empty() {
                let d = Diagnostic::error(
                    "E0111",
                    name_span,
                    format!(
                        "`enum {}` has no variant; an enum has at least one, as in `Empty` on a line of its own",
                        name
                    ),
                );
                self.error(d);
            }
        }
        // A variant's types nest only through the parser's recursion, whose depth limit reports
        // `E0127` first, so the tree is never deeper than the limit.
        EnumSyntax {
            name,
            name_span,
            variants,
            tree: std::mem::take(&mut self.tree),
        }
    }

    /// One variant, `name [ "(" field ":" type { "," field ":" type } ")" ]`. A payload of
    /// unnamed types, as in Rust's `Deposit(i64)`, or in braces, as in `Rect { w: u8 }`, is
    /// `E0130`, whose fix names the fields.
    fn variant(&mut self, variants: &mut Vec<VariantSyntax>) {
        // An attribute above a variant, or before it on its line, is read and left aside, as a
        // field's is (`grammar.md`, item 10).
        self.skip_attributes();
        if self.recovering {
            return;
        }
        let (name, name_span) = self.name("a variant's name");
        if self.recovering {
            return;
        }
        let mut fields = Vec::new();
        let braced = self.at(&TokenKind::LBrace) && !self.newline_before();
        if (self.at(&TokenKind::LParen) || braced)
            && !self.newline_before()
            && !self.payload(&name, braced, &mut fields)
        {
            return;
        }
        variants.push(VariantSyntax {
            name,
            name_span,
            fields,
        });
        if self.at(&TokenKind::Eq) && !self.newline_before() {
            let span = self.span();
            self.later(span, "a variant's written tag value", None);
            return;
        }
        self.comma_after("a variant");
    }

    /// Reads the attributes before a field or a variant, `@name` or `@name(...)`, above it or
    /// before it on its line, and leaves them aside.
    fn skip_attributes(&mut self) {
        while self.at(&TokenKind::At) && !self.recovering {
            self.bump();
            self.name("an attribute's name");
            if self.at(&TokenKind::LParen) {
                let close = self.closing_paren();
                while self.pos <= close && !self.at(&TokenKind::Eof) {
                    self.bump();
                }
            }
        }
    }

    /// A variant's payload, `(` to `)`, or another language's `{` to `}`: its fields, each
    /// `name: type`. Reports a payload written otherwise once, with the fix that writes it so.
    /// False after an error that ends the variant's line.
    fn payload(&mut self, variant: &str, braced: bool, fields: &mut Vec<FieldSyntax>) -> bool {
        let open = self.bump().span;
        let close = if braced {
            TokenKind::RBrace
        } else {
            TokenKind::RParen
        };
        // Each field written without its name, by where its type starts.
        let mut unnamed: Vec<Span> = Vec::new();
        while !self.at(&close) && !self.recovering && !self.at(&TokenKind::Eof) {
            let before = self.pos;
            let named = matches!(self.peek(), TokenKind::Ident(_))
                && self.peek_second() == &TokenKind::Colon
                && !matches!(
                    self.tokens.get(self.pos + 2).map(|t| &t.kind),
                    Some(TokenKind::Colon)
                );
            if named {
                let (fname, fspan) = self.name("a payload field's name");
                self.bump(); // `:`
                let ty = self.ty();
                fields.push(FieldSyntax {
                    is_pub: false,
                    name: fname,
                    name_span: fspan,
                    ty,
                });
            } else {
                let at = self.span();
                let ty = self.ty();
                unnamed.push(at);
                let generated = format!("value{}", unnamed.len() - 1);
                fields.push(FieldSyntax {
                    is_pub: false,
                    name: generated,
                    name_span: at,
                    ty,
                });
            }
            if self.pos == before {
                break;
            }
            if !self.recovering && !self.eat(&TokenKind::Comma) && !self.at(&close) {
                self.expected("`,` or the end of the payload's fields");
            }
        }
        if self.recovering {
            return false;
        }
        let end = self.span();
        if !self.expect(&close, "the end of the payload's fields") {
            return false;
        }
        // A payload has a field at least: a variant that holds nothing has no parentheses.
        if fields.is_empty() && !braced {
            let d = Diagnostic::error(
                "E0111",
                Span {
                    end: end.end,
                    ..open
                },
                format!(
                    "a variant's payload has at least one field, and a variant that holds nothing is written without parentheses: `{}`",
                    variant
                ),
            )
            .with_fix(
                Applicability::MachineApplicable,
                "remove the empty parentheses",
                Span {
                    end: end.end,
                    ..open
                },
                "",
            );
            self.error(d);
            self.recovering = false;
            return true;
        }
        if unnamed.is_empty() && !braced {
            return true;
        }
        // The names the fix gives: `value` for one field, `value0`, `value1` and on for several.
        let names: Vec<String> = if unnamed.len() == 1 {
            vec![String::from("value")]
        } else {
            (0..unnamed.len()).map(|k| format!("value{}", k)).collect()
        };
        let mut edits: Vec<crate::diagnostics::Edit> = unnamed
            .iter()
            .zip(&names)
            .map(|(at, n)| crate::diagnostics::Edit {
                span: Span {
                    end: at.start,
                    ..*at
                },
                replacement: format!("{}: ", n),
            })
            .collect();
        if braced {
            edits.insert(
                0,
                crate::diagnostics::Edit {
                    span: open,
                    replacement: "(".into(),
                },
            );
            edits.push(crate::diagnostics::Edit {
                span: end,
                replacement: ")".into(),
            });
        }
        let message = if braced && unnamed.is_empty() {
            format!(
                "a variant's payload is written in parentheses, its fields named, as in `{}(field: u8)`, not in braces",
                variant
            )
        } else {
            format!(
                "a variant's payload names its fields, as in `{}(amount: i64)`, so a value is built and matched by name",
                variant
            )
        };
        let d = Diagnostic::error(
            "E0130",
            Span {
                end: end.end,
                ..open
            },
            message,
        );
        let mut d = d;
        d.fixes.push(crate::diagnostics::Fix {
            message: if unnamed.is_empty() {
                "write the payload in parentheses".into()
            } else {
                "name the fields".into()
            },
            applicability: if unnamed.is_empty() {
                Applicability::MachineApplicable
            } else {
                Applicability::Suggestion
            },
            edits,
        });
        self.error(d);
        // The payload is read whole, to its closing bracket, so the variant goes on as one
        // whose fields are named, and the next line is the next variant's.
        self.recovering = false;
        true
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
        if !self.deeper() {
            return self.tree.push_type(TypeKind::Error, start);
        }
        let t = self.ty_within();
        self.depth -= 1;
        t
    }

    fn ty_within(&mut self) -> TypeId {
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
                    // A count, which takes no type: `[300_i8]i32` names a type it lies outside.
                    TokenKind::IntLit(_, raw) if int_suffix(&raw).is_some() => {
                        self.bad_type(
                            "an array's length is an integer literal without a suffix, as in `[4]u8`"
                                .into(),
                        );
                        return self.tree.push_type(TypeKind::Error, start);
                    }
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
                if let Some((m, what)) = later_type(&n) {
                    let what = format!("{} (`{}`)", what, n);
                    self.later(start, &what, Some(m));
                    return self.tree.push_type(TypeKind::Error, start);
                }
                // Any other name names a declaration of the module, which the checker finds;
                // a fixed-point type such as `q16.16` is none.
                self.bump();
                if self.at(&TokenKind::Dot) && !self.newline_before() {
                    let what = format!("a fixed-point type (`{}.`)", n);
                    self.later(start, &what, Some("M5"));
                    return self.tree.push_type(TypeKind::Error, start);
                }
                if self.at(&TokenKind::Lt) && !self.newline_before() {
                    let span = self.span();
                    self.later(span, "a type argument list", Some("M4"));
                    return self.tree.push_type(TypeKind::Error, start);
                }
                self.tree.push_type(TypeKind::Named(n), start)
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
        if !self.deeper() {
            return self.tree.push_block(Block {
                stmts: Vec::new(),
                span: open,
            });
        }
        let b = self.block_within();
        self.depth -= 1;
        b
    }

    fn block_within(&mut self) -> BlockId {
        let open = self.span();
        if !self.expect(&TokenKind::LBrace, "`{`") {
            return self.tree.push_block(Block {
                stmts: Vec::new(),
                span: open,
            });
        }
        let saved_nest = self.nest;
        let saved_cond = self.cond_nest.take();
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
                self.statement_end("a statement");
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.nest = saved_nest;
        self.cond_nest = saved_cond;
        let span = self.span_from(open);
        self.tree.push_block(Block { stmts, span })
    }

    /// After a statement, or a field of a struct, the next one starts on a new line (`E0112`).
    fn statement_end(&mut self, what: &str) {
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
                "{} ends at the end of its line, and {} starts another on the same line",
                what,
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
            TokenKind::Ident(w)
                if w == "let"
                    && matches!(self.peek_second(), TokenKind::Ident(_) | TokenKind::Mut) =>
            {
                self.let_binding()
            }
            TokenKind::Ident(w) if w == "_" && self.peek_second() == &TokenKind::Eq => {
                self.bump();
                self.bump();
                StmtKind::Discard(self.expr())
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
                                    "`{}` is not a place to assign to; a place is a name, or an element or a field of one such as `a[i]` or `s.f`",
                                    self.text_of(place)
                                ),
                            )
                            .with_related(op_span, "assigned here");
                            self.error(d);
                            // An error stops its statement's parse, as everywhere, so that no
                            // idiom in the value is read while recovering.
                            return self
                                .tree
                                .push_stmt(StmtKind::Expr(target), self.span_from(start));
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
        self.binding_rest(is_var)
    }

    /// `let` and `let mut` from Rust, read as `val` and `var` after their diagnostic (`E0118`).
    fn let_binding(&mut self) -> StmtKind {
        let first = self.bump().span;
        let is_var = self.at(&TokenKind::Mut);
        let span = if is_var {
            Span {
                end: self.bump().span.end,
                ..first
            }
        } else {
            first
        };
        let (mente, written) = if is_var {
            ("var", "let mut")
        } else {
            ("val", "let")
        };
        let d = Diagnostic::error(
            "E0118",
            span,
            format!("Mente writes `{}`, not `{}`", mente, written),
        )
        .with_fix(
            Applicability::MachineApplicable,
            format!("write `{}`", mente),
            span,
            mente,
        );
        self.note(d);
        self.binding_rest(is_var)
    }

    fn binding_rest(&mut self, is_var: bool) -> StmtKind {
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
        let outer = self.cond_nest.replace(self.nest);
        let e = self.binary(0);
        self.cond_nest = outer;
        e
    }

    fn is_place(&self, e: ExprId) -> bool {
        match &self.tree.expr(e).kind {
            ExprKind::Name(_) => true,
            ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => self.is_place(*base),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- expressions

    /// A full expression, which may be an `if` (`grammar.md`, item 7).
    fn expr(&mut self) -> ExprId {
        let start = self.span();
        if !self.deeper() {
            return self.tree.push_expr(ExprKind::Error, start);
        }
        let e = self.expr_within();
        self.depth -= 1;
        e
    }

    fn expr_within(&mut self) -> ExprId {
        let e = match self.peek() {
            TokenKind::If => self.if_expr(),
            TokenKind::Match => self.match_expr(),
            _ => return self.binary(0),
        };
        if !self.recovering && !self.ends_here() && infix(self.peek()).is_some() {
            self.if_in_operand(e);
        }
        e
    }

    /// An `if` or a `match` used as an operand needs parentheses (`E0115`); the fix adds them.
    fn if_in_operand(&mut self, e: ExprId) {
        let span = self.tree.expr(e).span;
        let word = match self.tree.expr(e).kind {
            ExprKind::Match { .. } => "match",
            _ => "if",
        };
        let article = if word == "if" { "an" } else { "a" };
        let wrapped = format!("({})", self.text_of(span));
        let d = Diagnostic::error(
            "E0115",
            span,
            format!(
                "{} `{}` expression used as an operand needs parentheses",
                article, word
            ),
        )
        .with_fix(
            Applicability::MachineApplicable,
            format!("put the `{}` expression in parentheses", word),
            span,
            wrapped,
        );
        self.error(d);
    }

    // ---------------------------------------------------------------- match

    /// `match scrutinee { arm ... }`, one arm per line (`grammar.md`, item 12).
    fn match_expr(&mut self) -> ExprId {
        let start = self.span();
        self.bump(); // `match`
        let scrutinee = self.cond();
        let mut arms = Vec::new();
        if self.recovering {
            return self.tree.push_expr(ExprKind::Error, self.span_from(start));
        }
        let open = self.span();
        if !self.expect(&TokenKind::LBrace, "`{` and the arms of the `match`") {
            return self.tree.push_expr(ExprKind::Error, self.span_from(start));
        }
        // Inside the braces, as in a block, a newline ends an arm.
        let saved_nest = self.nest;
        let saved_cond = self.cond_nest.take();
        let saved_index = self.index_nest.take();
        self.nest = 0;
        loop {
            match self.peek() {
                TokenKind::RBrace => {
                    self.bump();
                    break;
                }
                TokenKind::Eof => {
                    let d = Diagnostic::error("E0111", open, "this `match` has no closing `}`");
                    self.error(d);
                    break;
                }
                _ => {}
            }
            let before = self.pos;
            if let Some(arm) = self.arm() {
                arms.push(arm);
            }
            if self.recovering {
                self.skip_statement();
            } else {
                self.statement_end("an arm");
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.nest = saved_nest;
        self.cond_nest = saved_cond;
        self.index_nest = saved_index;
        if arms.is_empty() && self.errors.is_empty() {
            let d = Diagnostic::error(
                "E0111",
                start,
                "this `match` has no arm; a `match` has one for each value of what it reads, as in `0 => a` on a line of its own",
            );
            self.error(d);
        }
        let span = self.span_from(start);
        self.tree
            .push_expr(ExprKind::Match { scrutinee, arms }, span)
    }

    /// One arm, `pattern => expression` or `pattern => { ... }`. A newline after `=>` does not
    /// end it. An arm guard (`E0131`) and alternatives (`E0132`) are other languages' patterns.
    fn arm(&mut self) -> Option<Arm> {
        let start = self.span();
        let pattern = self.pattern()?;
        if self.at(&TokenKind::If) && !self.newline_before() {
            let at = self.span();
            let d = Diagnostic::error(
                "E0131",
                at,
                "an arm has no guard: its pattern alone decides whether it runs; test the condition with `if` inside the arm, whose other branch does what a later arm would",
            );
            self.error(d);
            return None;
        }
        if self.at(&TokenKind::Pipe) && !self.newline_before() {
            let at = self.span();
            let d = Diagnostic::error(
                "E0132",
                at,
                "a pattern matches one variant or one value, so it has no alternatives; write an arm for each, or `_` after them on an integer",
            );
            self.error(d);
            return None;
        }
        if !self.expect(&TokenKind::FatArrow, "`=>` after the arm's pattern") {
            return None;
        }
        let body = if self.at(&TokenKind::LBrace) {
            ArmBody::Block(self.block())
        } else {
            ArmBody::Expr(self.expr())
        };
        if self.recovering {
            return None;
        }
        // An assignment is a statement, which an arm holds only in a block.
        if let (ArmBody::Expr(target), Some(_)) = (body, assign_op(self.peek())) {
            if !self.ends_here() {
                let from = self.tree.expr(target).span;
                self.bump();
                self.expr();
                if self.recovering {
                    return None;
                }
                let whole = self.span_from(from);
                let text = format!("{{ {} }}", self.text_of(whole));
                let d = Diagnostic::error(
                    "E0111",
                    whole,
                    "an arm is an expression or a block, and an assignment is a statement, so it stands in a block",
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "put the assignment in a block",
                    whole,
                    text,
                );
                self.error(d);
                // The assignment is read whole, so the arm's line ends here and the next
                // line is the next arm's.
                self.recovering = false;
                return None;
            }
        }
        let span = self.span_from(start);
        self.comma_after("an arm");
        Some(Arm {
            pattern,
            body,
            span,
        })
    }

    /// A pattern of M2: a variant with its bindings, an integer literal with `-` or not,
    /// `true`, `false`, or `_`. A pattern of `?T`, `none` or a name, comes later in M2 (`E0109`),
    /// and one M2 does not have, such as a range or a tuple, is `E0132`.
    fn pattern(&mut self) -> Option<Pattern> {
        let start = self.span();
        let kind = match self.peek().clone() {
            TokenKind::Ident(n) if n == "_" => {
                self.bump();
                PatternKind::Wildcard
            }
            TokenKind::Ident(n) => {
                self.bump();
                if self.at(&TokenKind::Colon)
                    && self.peek_second() == &TokenKind::Colon
                    && self.next_two_touch()
                {
                    self.path_idiom();
                    return None;
                }
                if !self.at(&TokenKind::Dot) || self.newline_before() {
                    let what = format!(
                        "a name pattern (`{}`), which binds the present value of a `?T`,",
                        n
                    );
                    self.later(start, &what, Some("M2"));
                    return None;
                }
                self.bump(); // `.`
                let (name, name_span) = self.name("a variant's name after `.`");
                if self.recovering {
                    return None;
                }
                let bindings = if self.at(&TokenKind::LParen) && !self.newline_before() {
                    Some(self.pattern_bindings()?)
                } else {
                    None
                };
                PatternKind::Variant {
                    enum_name: n,
                    name,
                    name_span,
                    bindings,
                }
            }
            TokenKind::Minus => {
                self.bump();
                match self.peek().clone() {
                    TokenKind::IntLit(value, raw) => {
                        self.bump();
                        PatternKind::Int {
                            negative: true,
                            value,
                            suffix: int_suffix(&raw),
                        }
                    }
                    _ => {
                        self.expected("an integer literal after `-` in a pattern");
                        return None;
                    }
                }
            }
            TokenKind::IntLit(value, raw) => {
                self.bump();
                PatternKind::Int {
                    negative: false,
                    value,
                    suffix: int_suffix(&raw),
                }
            }
            TokenKind::TrueLit => {
                self.bump();
                PatternKind::Bool(true)
            }
            TokenKind::FalseLit => {
                self.bump();
                PatternKind::Bool(false)
            }
            TokenKind::NoneLit => {
                self.later(
                    start,
                    "`none`, the pattern of an absent `?T` value,",
                    Some("M2"),
                );
                return None;
            }
            TokenKind::LParen | TokenKind::LBracket => {
                let d = Diagnostic::error(
                    "E0132",
                    start,
                    "a pattern is a variant, an integer literal, `true`, `false` or `_`, never a tuple or an array of patterns; match the parts with a `match` each",
                );
                self.error(d);
                return None;
            }
            _ => {
                self.expected("a pattern: a variant such as `Shape.Dot`, an integer literal, `true`, `false` or `_`");
                return None;
            }
        };
        if matches!(self.peek(), TokenKind::DotDot | TokenKind::DotDotEq) && !self.newline_before()
        {
            let at = self.span();
            let d = Diagnostic::error(
                "E0132",
                at,
                "a pattern matches one value, never a range; write an arm for each value, or test the range with `if` in an arm after `_`",
            );
            self.error(d);
            return None;
        }
        Some(Pattern {
            kind,
            span: self.span_from(start),
        })
    }

    /// A variant pattern's bindings, `(field: name, other: _)`. A binding is a name or `_`: a
    /// pattern in its place nests, which M2's patterns do not (`E0132`).
    fn pattern_bindings(&mut self) -> Option<Vec<PatBinding>> {
        self.bump(); // `(`
        self.nest += 1;
        let mut bindings = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.recovering && !self.at(&TokenKind::Eof) {
            let field = if self.named_argument_follows() {
                let (f, fspan) = self.name("a field's name");
                self.bump(); // `:`
                Some((f, fspan))
            } else {
                None
            };
            let at = self.span();
            let ends = |k: Option<&Token>| {
                matches!(
                    k.map(|t| &t.kind),
                    Some(TokenKind::Comma | TokenKind::RParen)
                )
            };
            match self.peek().clone() {
                TokenKind::Ident(n) if ends(self.tokens.get(self.pos + 1)) => {
                    self.bump();
                    bindings.push(PatBinding {
                        field,
                        name: (n != "_").then_some(n),
                        name_span: at,
                    });
                }
                _ => {
                    let d = Diagnostic::error(
                        "E0132",
                        at,
                        "a payload's field binds a name or `_`, never a pattern of its own: bind it to a name, and `match` that name in the arm",
                    );
                    self.error(d);
                    break;
                }
            }
            if !self.recovering && !self.eat(&TokenKind::Comma) && !self.at(&TokenKind::RParen) {
                self.expected("`,` or `)` after a binding");
            }
        }
        self.nest -= 1;
        if self.recovering {
            return None;
        }
        if !self.expect(&TokenKind::RParen, "`)` to close the bindings") {
            return None;
        }
        Some(bindings)
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
            if let Some((op, word)) = self.c_logical() {
                let lbp = if op == BinaryOp::And { 2 } else { 1 };
                if lbp < min_bp {
                    break;
                }
                let first = self.bump().span;
                let second = self.bump().span;
                let span = Span {
                    end: second.end,
                    ..first
                };
                let d = Diagnostic::error(
                    "E0124",
                    span,
                    format!("Mente writes `{}`, not `{}`", word, self.text_of(span)),
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    format!("write `{}`", word),
                    span,
                    word,
                );
                self.note(d);
                let rhs = self.binary(lbp + 1);
                let span = self.span_from(start);
                lhs = self.tree.push_expr(ExprKind::Binary { op, lhs, rhs }, span);
                continue;
            }
            if self.at(&TokenKind::As) {
                self.cast(lhs);
                continue;
            }
            if self.at(&TokenKind::Question) && self.ternary_follows() {
                if min_bp > 0 {
                    break;
                }
                self.ternary(&mut lhs, start);
                continue;
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

    /// `&&` or `||` from C, Rust and JavaScript, as two touching tokens.
    fn c_logical(&self) -> Option<(BinaryOp, &'static str)> {
        if !self.next_two_touch() {
            return None;
        }
        match (self.peek(), self.peek_second()) {
            (TokenKind::Amp, TokenKind::Amp) => Some((BinaryOp::And, "and")),
            (TokenKind::Pipe, TokenKind::Pipe) => Some((BinaryOp::Or, "or")),
            _ => None,
        }
    }

    /// `x as T` from Rust and C (`E0126`): Mente has no casts.
    fn cast(&mut self, value: ExprId) {
        let from = self.tree.expr(value).span.end;
        let as_span = self.bump().span;
        // The type is read only to find where it ends; its own errors would be a second report.
        let was = self.recovering;
        self.recovering = true;
        self.ty();
        self.recovering = was;
        let removed = Span {
            start: from,
            end: self.prev_end(),
            ..as_span
        };
        let d = Diagnostic::error(
            "E0126",
            as_span,
            "Mente has no casts: an integer converts where it lands, checked there, so bind it with the type it needs, as in `val y: u8 = x`",
        )
        .with_fix(Applicability::Suggestion, "remove the cast", removed, "");
        self.note(d);
    }

    /// Whether the `?` at the next token opens C's conditional `c ? a : b`: a value follows it
    /// on its line.
    fn ternary_follows(&self) -> bool {
        let next = self.second();
        next.span.line == self.span().line && starts_expression(&next.kind)
    }

    /// `c ? a : b` from C and JavaScript (`E0125`), read as `if c { a } else { b }` after its
    /// diagnostic, whose fix writes it so.
    fn ternary(&mut self, cond: &mut ExprId, start: Span) {
        let question = self.bump().span;
        let then_value = self.binary(0);
        if self.recovering || !self.at(&TokenKind::Colon) {
            self.later(question, "error propagation (`?` and `!`)", Some("M2"));
            return;
        }
        self.bump();
        let else_value = self.binary(0);
        if self.recovering {
            return;
        }
        let whole = self.span_from(start);
        let rewritten = format!(
            "if {} {{ {} }} else {{ {} }}",
            self.text_of(self.tree.expr(*cond).span),
            self.text_of(self.tree.expr(then_value).span),
            self.text_of(self.tree.expr(else_value).span)
        );
        let d = Diagnostic::error(
            "E0125",
            question,
            "Mente has no `? :`; write `if c { a } else { b }`",
        )
        .with_fix(
            Applicability::MachineApplicable,
            "write it as an `if` expression",
            whole,
            rewritten,
        );
        self.note(d);
        let then = self.single(then_value);
        let otherwise = Some(Else::Block(self.single(else_value)));
        *cond = self.tree.push_expr(
            ExprKind::If {
                cond: *cond,
                then,
                otherwise,
            },
            whole,
        );
    }

    /// A block holding one expression, as an `if` read from `? :` has.
    fn single(&mut self, value: ExprId) -> BlockId {
        let span = self.tree.expr(value).span;
        let stmt = self.tree.push_stmt(StmtKind::Expr(value), span);
        self.tree.push_block(Block {
            stmts: vec![stmt],
            span,
        })
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
                if self.at(&TokenKind::DotDot) {
                    let span = self.span();
                    self.later(span, "a view `x[..]`", Some("M2"));
                    return true;
                }
                self.nest += 1;
                let outer = self.index_nest.replace(self.nest);
                let index = self.expr();
                self.index_nest = outer;
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
                let through_name = Span {
                    end: name_span.end,
                    ..at
                };
                if name == "await" && !self.at(&TokenKind::LParen) {
                    let d = Diagnostic::error(
                        "E0121",
                        through_name,
                        "Mente has no `await`: a function that waits declares its effects with `with`, and tasks run at once with `parallel (...)`",
                    )
                    .with_fix(Applicability::Suggestion, "remove `.await`", through_name, "");
                    self.note(d);
                    return true;
                }
                if (name == "unwrap" || name == "expect") && self.at(&TokenKind::LParen) {
                    let close = self.closing_paren();
                    while self.pos < close {
                        self.bump();
                    }
                    self.bump();
                    let call = Span {
                        end: self.prev_end(),
                        ..at
                    };
                    let d = Diagnostic::error(
                        "E0122",
                        through_name,
                        format!(
                            "Mente has no `.{}()`: test the value with `guard`, or give a default with `??` (M2)",
                            name
                        ),
                    )
                    .with_fix(
                        Applicability::Suggestion,
                        format!("remove `.{}(...)`", name),
                        call,
                        "",
                    );
                    self.note(d);
                    return true;
                }
                if self.at(&TokenKind::LParen) && !self.newline_before() {
                    self.later(name_span, "a method call", Some("M2"));
                } else {
                    let span = self.span_from(start);
                    *lhs = self
                        .tree
                        .push_expr(ExprKind::Field { base: *lhs, name }, span);
                }
                true
            }
            // `c ? a : b` is read in `binary`, at the level of a whole expression.
            TokenKind::Question if self.ternary_follows() => false,
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
            // Directly inside an index's brackets a range makes a view, which M2 brings.
            TokenKind::DotDot | TokenKind::DotDotEq if self.index_nest == Some(self.nest) => {
                self.later(at, "a range `x[a..b]`", Some("M2"));
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
                if self.at(&TokenKind::Colon)
                    && self.peek_second() == &TokenKind::Colon
                    && self.next_two_touch()
                {
                    self.path_idiom();
                    ExprKind::Error
                } else if n == "await" && !self.newline_before() && starts_expression(self.peek()) {
                    let removed = Span {
                        end: self.span().start,
                        ..start
                    };
                    let d = Diagnostic::error(
                        "E0121",
                        start,
                        "Mente has no `await`: a function that waits declares its effects with `with`, and tasks run at once with `parallel (...)`",
                    )
                    .with_fix(Applicability::Suggestion, "remove `await`", removed, "");
                    self.note(d);
                    return self.binary(UNARY_OPERAND_BP);
                } else if n == "async" && self.at(&TokenKind::LBrace) {
                    let d = Diagnostic::error(
                        "E0121",
                        start,
                        "Mente has no `async` blocks: tasks run at once with `parallel (...)` (M7)",
                    );
                    self.error(d);
                    ExprKind::Error
                } else if self.at(&TokenKind::LParen) && !self.newline_before() {
                    if n == "todo" || n == "unreachable" {
                        self.marker(&n)
                    } else {
                        self.call(n)
                    }
                } else if self.dotted_call_follows() {
                    self.dotted_call(n, start)
                } else if self.dotted_braced_follows() {
                    self.dotted_braced(n, start)
                } else if self.struct_literal_follows() {
                    self.struct_literal(n, start)
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
            TokenKind::Match => {
                let e = self.match_expr();
                if !self.recovering {
                    self.if_in_operand(e);
                }
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
            TokenKind::NoneLit | TokenKind::Raise | TokenKind::Catch => {
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
            TokenKind::Amp => return self.reference(),
            TokenKind::Bang => {
                let bang = self.bump().span;
                let operand = self.binary(UNARY_OPERAND_BP);
                if !self.recovering {
                    let operand_span = self.tree.expr(operand).span;
                    let text = format!("not {}", self.text_of(operand_span));
                    // `!a == b` is `(!a) == b`, but `not a == b` is `not (a == b)`.
                    let tight = !self.ends_here() && infix(self.peek()).is_some();
                    let replacement = if tight { format!("({})", text) } else { text };
                    let d = Diagnostic::error("E0124", bang, "Mente writes `not`, not `!`")
                        .with_fix(
                            Applicability::MachineApplicable,
                            "write `not`",
                            Span {
                                end: operand_span.end,
                                ..bang
                            },
                            replacement,
                        );
                    self.note(d);
                }
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand,
                }
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

    /// `a::b` and `f::<T>` from Rust and C++ (`E0123`).
    fn path_idiom(&mut self) {
        let first = self.bump().span;
        let second = self.bump().span;
        let colons = Span {
            end: second.end,
            ..first
        };
        let d = if self.at(&TokenKind::Lt) {
            let lt = self.span();
            Diagnostic::error(
                "E0123",
                colons,
                "Mente writes type arguments as `f<T>(...)`, without `::`",
            )
            .with_fix(
                Applicability::MachineApplicable,
                "write `<`",
                Span {
                    end: lt.end,
                    ..colons
                },
                "<",
            )
        } else {
            Diagnostic::error(
                "E0123",
                colons,
                "Mente separates the parts of a path with `.`, not `::`",
            )
            .with_fix(Applicability::Suggestion, "write `.`", colons, ".")
        };
        self.error(d);
    }

    /// `&x` and `&mut x` from Rust (`E0119`): Mente has no references. Read as `x` after the
    /// diagnostic.
    fn reference(&mut self) -> ExprId {
        let amp = self.bump().span;
        let is_mut = self.eat(&TokenKind::Mut);
        let written = Span {
            end: self.prev_end(),
            ..amp
        };
        let removed = Span {
            end: self.span().start,
            ..amp
        };
        let (message, fix) = if is_mut {
            (
                "Mente has no references: a call passes a place to change as `mut x`, and nothing else takes `&mut x`",
                "remove `&mut`",
            )
        } else {
            (
                "Mente has no references: a parameter is read-only unless it is `mut`, so pass `x`, not `&x`",
                "remove `&`",
            )
        };
        let d = Diagnostic::error("E0119", written, message).with_fix(
            Applicability::Suggestion,
            fix,
            removed,
            "",
        );
        self.note(d);
        self.binary(UNARY_OPERAND_BP)
    }

    fn unary(&mut self, op: UnaryOp, operand_bp: u8) -> ExprKind {
        self.bump();
        if !self.deeper() {
            return ExprKind::Error;
        }
        let operand = self.binary(operand_bp);
        self.depth -= 1;
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
                // Each `else if` nests one level deeper.
                let start = self.span();
                if self.deeper() {
                    let e = self.if_expr();
                    self.depth -= 1;
                    Else::If(e)
                } else {
                    Else::If(self.tree.push_expr(ExprKind::Error, start))
                }
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

    /// Whether the next tokens begin a named argument, `name:` (`grammar.md`, item 12), and not
    /// the `::` of a path.
    fn named_argument_follows(&self) -> bool {
        let third = &self.tokens[(self.pos + 2).min(self.tokens.len() - 1)];
        matches!(self.peek(), TokenKind::Ident(_))
            && self.peek_second() == &TokenKind::Colon
            && !(third.kind == TokenKind::Colon && third.span.start == self.second().span.end)
    }

    /// The named arguments of a struct being built, up to `close`: `name: value`, separated by
    /// commas. Arguments are positional or named, never both in one list.
    fn field_inits(&mut self, close: &TokenKind) -> Vec<FieldInit> {
        let mut fields = Vec::new();
        while !self.at(close) && !self.recovering {
            let before = self.pos;
            if !self.named_argument_follows() {
                let d = Diagnostic::error(
                    "E0111",
                    self.span(),
                    format!(
                        "expected a field's name and `:`, found {}; the arguments that build a struct are all named, as in `Pair(i: 0, j: 1)`",
                        self.found()
                    ),
                );
                self.error(d);
                break;
            }
            let (name, name_span) = self.name("a field's name");
            self.bump(); // `:`
            let value = self.expr();
            fields.push(FieldInit {
                name,
                name_span,
                value,
            });
            if self.pos == before {
                break;
            }
            if !self.recovering && !self.eat(&TokenKind::Comma) && !self.at(close) {
                self.expected("`,` or the end of the fields");
            }
        }
        fields
    }

    /// Whether a `{` after a name opens another language's struct literal, `Name { f: e }`: it
    /// stands on the name's line and holds `field:`, and the name does not end a condition, where
    /// the `{` opens the block that follows.
    fn struct_literal_follows(&self) -> bool {
        if !self.at(&TokenKind::LBrace) || self.newline_before() {
            return false;
        }
        if self.cond_nest == Some(self.nest) {
            return false;
        }
        let at = |k: usize| self.tokens.get(self.pos + k).map(|t| &t.kind);
        matches!(at(1), Some(TokenKind::Ident(_))) && at(2) == Some(&TokenKind::Colon)
    }

    /// Rust's struct literal `Name { f: e }` (`E0129`), read as `Name(f: e)` after its
    /// diagnostic, whose fix writes it so.
    fn struct_literal(&mut self, name: String, start: Span) -> ExprKind {
        let what = format!(
            "a struct with its fields in parentheses, as in `{}(field: value)`",
            name
        );
        match self.braced_fields(&what, start, start.end) {
            Some(fields) => ExprKind::Construct { name, fields },
            None => ExprKind::Error,
        }
    }

    /// Whether the next tokens, after a name, are `.name {` on one line holding `field:`: a
    /// variant built as another language's struct literal, `Name.B { f: e }`.
    fn dotted_braced_follows(&self) -> bool {
        let at = |k: usize| self.tokens.get(self.pos + k);
        let (Some(dot), Some(name), Some(open), Some(field), Some(colon)) =
            (at(0), at(1), at(2), at(3), at(4))
        else {
            return false;
        };
        dot.kind == TokenKind::Dot
            && !self.ends_here()
            && matches!(name.kind, TokenKind::Ident(_))
            && open.kind == TokenKind::LBrace
            && open.span.line == name.span.line
            && matches!(field.kind, TokenKind::Ident(_))
            && colon.kind == TokenKind::Colon
            && self.cond_nest != Some(self.nest)
    }

    /// A variant built in braces, `Name.B { f: e }` (`E0129`), read as `Name.B(f: e)` after
    /// its diagnostic, whose fix writes it so, as a struct literal in braces is.
    fn dotted_braced(&mut self, enum_name: String, start: Span) -> ExprKind {
        self.bump(); // `.`
        let (name, name_span) = self.name("a variant's name");
        let what = format!(
            "a variant's payload with its fields in parentheses, as in `{}.{}(field: value)`",
            enum_name, name
        );
        match self.braced_fields(&what, start, name_span.end) {
            Some(fields) => ExprKind::Variant {
                enum_name,
                enum_span: start,
                name,
                name_span,
                payload: Payload::Named(fields),
            },
            None => ExprKind::Error,
        }
    }

    /// The fields of another language's literal in braces, from `{` to `}`, after a name that
    /// starts at `start` and ends at byte `name_end`: `E0129`, which says Mente builds `what`,
    /// with the fix that writes the fields in parentheses. `None` after an error among them.
    fn braced_fields(
        &mut self,
        what: &str,
        start: Span,
        name_end: usize,
    ) -> Option<Vec<FieldInit>> {
        self.bump();
        // The first field's name, where the `(` that replaces `{` ends.
        let first = self.span().start;
        self.nest += 1;
        let fields = self.field_inits(&TokenKind::RBrace);
        self.nest -= 1;
        // The fields end at `}`, or at an error `field_inits` reported.
        if self.recovering {
            return None;
        }
        let close = self.bump().span;
        let mut d = Diagnostic::error(
            "E0129",
            Span {
                end: close.end,
                ..start
            },
            format!("Mente builds {}, not in braces", what),
        );
        d.fixes.push(crate::diagnostics::Fix {
            message: "write the fields in parentheses".into(),
            applicability: Applicability::MachineApplicable,
            edits: vec![
                crate::diagnostics::Edit {
                    span: Span {
                        start: name_end,
                        end: first,
                        ..start
                    },
                    replacement: "(".into(),
                },
                crate::diagnostics::Edit {
                    span: Span {
                        start: self.tokens[self.pos - 2].span.end,
                        ..close
                    },
                    replacement: ")".into(),
                },
            ],
        });
        self.note(d);
        Some(fields)
    }

    /// Whether the next tokens, after a name, are `.name(` on one line: a variant built, or a
    /// call on a receiver, which name resolution tells apart (`grammar.md`, item 12). `.unwrap()`
    /// and `.expect()` are another language's, which [`Self::postfix`] reports.
    fn dotted_call_follows(&self) -> bool {
        let at = |k: usize| self.tokens.get(self.pos + k);
        let (Some(dot), Some(name), Some(open)) = (at(0), at(1), at(2)) else {
            return false;
        };
        dot.kind == TokenKind::Dot
            && !self.ends_here()
            && matches!(&name.kind, TokenKind::Ident(m) if m != "unwrap" && m != "expect")
            && open.kind == TokenKind::LParen
            && open.span.line == name.span.line
    }

    /// `a.b(...)`, `a` a name: a variant built with its payload, or, as name resolution finds
    /// `a`, a call on a receiver.
    fn dotted_call(&mut self, enum_name: String, start: Span) -> ExprKind {
        self.bump(); // `.`
        let (name, name_span) = self.name("a variant's name after `.`");
        // A type of the language is no enum of the module: a call on it, such as
        // `List.new()`, is a construct of a later milestone, whatever name resolution finds.
        if language_type(&enum_name) {
            self.later(name_span, "a method call", Some("M2"));
            return ExprKind::Error;
        }
        // After an error among the arguments, the statement is skipped from their `)`, so that
        // the brackets after it are counted from where they open.
        let close = self.closing_paren();
        let payload = match self.arguments() {
            Arguments::Named(fields) => Payload::Named(fields),
            Arguments::Positional(args) => Payload::Positional(args),
        };
        // Recovery leaves the parser at or before the `)`, or at the end of the file.
        if self.recovering {
            self.pos = (close + 1).min(self.tokens.len() - 1);
        }
        ExprKind::Variant {
            enum_name,
            enum_span: start,
            name,
            name_span,
            payload,
        }
    }

    fn call(&mut self, callee: String) -> ExprKind {
        match self.arguments() {
            // A list whose first argument is named builds a struct.
            Arguments::Named(fields) => ExprKind::Construct {
                name: callee,
                fields,
            },
            Arguments::Positional(args) => ExprKind::Call { callee, args },
        }
    }

    /// An argument list, `(` to `)`: named, when its first argument is, or positional.
    fn arguments(&mut self) -> Arguments {
        self.bump(); // `(`
        self.nest += 1;
        if self.named_argument_follows() {
            let fields = self.field_inits(&TokenKind::RParen);
            if !self.recovering {
                self.bump(); // `)`
            }
            self.nest -= 1;
            return Arguments::Named(fields);
        }
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.recovering {
            let before = self.pos;
            if self.named_argument_follows() {
                let d = Diagnostic::error(
                    "E0111",
                    self.span(),
                    format!(
                        "expected an argument, found the field name {}; a function's arguments are all positional, and only those that build a struct are named",
                        self.found()
                    ),
                );
                self.error(d);
                break;
            }
            if self.at(&TokenKind::Amp) && self.peek_second() == &TokenKind::Mut {
                // A call's `&mut x` is Rust's spelling of `mut x`: the rewrite is unique.
                let amp = self.bump().span;
                let span = Span {
                    end: self.span().end,
                    ..amp
                };
                let d = Diagnostic::error(
                    "E0119",
                    span,
                    "a call passes a place to change as `mut x`, not `&mut x`",
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "write `mut`",
                    span,
                    "mut",
                );
                self.note(d);
            }
            let is_mut = self.eat(&TokenKind::Mut);
            let value = self.expr();
            if is_mut && !self.recovering && !self.is_place(value) {
                let span = self.tree.expr(value).span;
                let d = Diagnostic::error(
                    "E0114",
                    span,
                    format!(
                        "after `mut`, an argument is a place, a name or an element or a field of one, not `{}`",
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
        Arguments::Positional(args)
    }

    /// `todo("reason")` or `unreachable("reason")`, whose argument is a string literal.
    fn marker(&mut self, name: &str) -> ExprKind {
        self.bump(); // `(`
        let reason = match (self.peek().clone(), self.peek_second()) {
            (TokenKind::StrLit(s), TokenKind::RParen) => s,
            _ => {
                let d = Diagnostic::error(
                    "E0111",
                    self.span(),
                    format!(
                        "`{}` takes its reason as a string, as in `{}(\"why\")`; found {}",
                        name,
                        name,
                        self.found()
                    ),
                );
                self.error(d);
                return ExprKind::Error;
            }
        };
        self.bump();
        self.bump();
        if name == "todo" {
            ExprKind::Todo(reason)
        } else {
            ExprKind::Unreachable(reason)
        }
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

/// An argument list, as written.
enum Arguments {
    Named(Vec<FieldInit>),
    Positional(Vec<Arg>),
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

/// Whether a token can start an expression, which tells C's `c ? a : b` and `await x` apart.
fn starts_expression(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::IntLit(..)
            | TokenKind::FloatLit(_)
            | TokenKind::StrLit(_)
            | TokenKind::Ident(_)
            | TokenKind::TrueLit
            | TokenKind::FalseLit
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::Minus
            | TokenKind::Tilde
            | TokenKind::Not
            | TokenKind::Bang
            | TokenKind::If
            | TokenKind::Match
    )
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

    /// A struct declaration, `grammar.md` item 10.
    fn parse_struct_src(src: &str) -> (StructSyntax, Vec<Diagnostic>) {
        let (tokens, errors) = lex(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();
        parse_struct(src, &tokens)
    }

    fn struct_codes(src: &str) -> Vec<&'static str> {
        parse_struct_src(src).1.iter().map(|d| d.code).collect()
    }

    /// A struct's fields stand one per line, each a name and a type of M2's grammar, a struct's
    /// name among them; attributes and doc comments above a field are read and left aside.
    #[test]
    fn test_struct_declarations_take_one_field_per_line() {
        let (s, errors) = parse_struct_src(
            "struct Seg {\n  /// The start.\n  pub from: Point\n  @bytes(0..4) len: u32[0..=100]\n  xs: [2][3]bool\n}",
        );
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(
            Tree::render_struct(&s),
            "(struct Seg (pub from Point) (len u32[0..=100]) (xs [2][3]bool))"
        );
        assert_eq!(s.fields[1].name, "len");
        // At least one field, each on its own line, with no comma after it.
        assert_eq!(struct_codes("struct E {\n}"), ["E0111"]);
        assert_eq!(struct_codes("struct P {\n  i: u8 j: u8\n}"), ["E0112"]);
        assert_eq!(struct_codes("struct P {\n  i u8\n}"), ["E0111"]);
        let src = "struct P {\n  i: u8,\n  j: u8\n}";
        let (s, errors) = parse_struct_src(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0128"]);
        assert_eq!(s.fields.len(), 2);
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "struct P {\n  i: u8\n  j: u8\n}"
        );
        // An invariant block is passed over to its closing brace, nested braces included, and
        // the fields after it are read; one left open ends at the end of the struct's tokens.
        for src in [
            "struct R {\n  invariant {\n    lo < 3\n  }\n  lo: u8\n}",
            "struct R {\n  invariant {\n    { lo }\n    {\n      { lo }\n    }\n  }\n  lo: u8\n}",
            "struct R {\n  invariant { lo < 3 }\n  lo: u8\n}",
        ] {
            let (s, errors) = parse_struct_src(src);
            assert_eq!(
                errors.iter().map(|d| d.code).collect::<Vec<_>>(),
                ["E0109"],
                "{}",
                src
            );
            assert_eq!(
                s.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
                ["lo"],
                "{}",
                src
            );
        }
        let (s, errors) = parse_struct_src("struct R {\n  lo: u8\n  invariant {\n    lo < 3\n");
        assert_eq!(errors[0].code, "E0109");
        assert_eq!(s.fields.len(), 1);
        // Type parameters, an invariant, a default and a type of a later milestone.
        for (src, message) in [
            (
                "struct B<T> {\n  v: u8\n}",
                "a struct with type parameters comes in M4; M2 does not have it",
            ),
            (
                "struct R {\n  lo: u8\n  invariant {\n    lo < 3\n  }\n}",
                "an invariant relating a struct's fields comes in a later milestone; M2 does not have it",
            ),
            (
                "struct C {\n  n: u8 = 0\n}",
                "a field's default value is in no milestone yet; M2 does not have it",
            ),
            (
                "struct N {\n  name: String\n}",
                "a string or list type (`String`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "struct F {\n  level: q16.16\n}",
                "a fixed-point type (`q16.`) comes in M5; M2 does not have it",
            ),
            (
                "struct H {\n  h: Handle<F>\n}",
                "an arena, pool or layout type (`Handle`) comes in M3; M2 does not have it",
            ),
            (
                "struct G {\n  g: Box<u8>\n}",
                "a type argument list comes in M4; M2 does not have it",
            ),
        ] {
            let errors: Vec<(&str, String)> = parse_struct_src(src)
                .1
                .into_iter()
                .map(|d| (d.code, d.message))
                .collect();
            assert_eq!(errors, vec![("E0109", String::from(message))], "{}", src);
        }
    }

    /// A call with named arguments builds a struct, one with positional arguments calls a
    /// function, and a list never mixes the two; a field of a place is a place.
    #[test]
    fn test_structs_are_built_with_named_arguments_and_fields_are_places() {
        assert_eq!(expr("Pair(i: 0, j: a + 1)"), "(new Pair (i 0) (j (+ a 1)))");
        assert_eq!(expr("Pair(i: 0,\n  j: 1,\n)"), "(new Pair (i 0) (j 1))");
        assert_eq!(
            expr("s.inner.x + xs[1].y"),
            "(+ (. (. s inner) x) (. (index xs 1) y))"
        );
        assert_eq!(expr("f(Pair(i: 0), 1)"), "(call f (new Pair (i 0)) 1)");
        assert_eq!(codes("val p = Pair(i: 0, 1)"), ["E0111"]);
        assert_eq!(codes("val p = f(1, i: 0)"), ["E0111"]);
        assert_eq!(
            block("s.f = 1\ns.f[i] = 2\nxs[i].f += 3\ng(mut s.a.b, mut xs[0].c)"),
            "{(= (. s f) 1); (= (index (. s f) i) 2); (+= (. (index xs i) f) 3); (call g mut (. (. s a) b) mut (. (index xs 0) c))}"
        );
        assert_eq!(codes("f().x = 1"), ["E0114"]);
        // A method call is still a later construct; a field may take any name.
        assert_eq!(codes("val n = s.f.size()"), ["E0109"]);
        assert_eq!(expr("s.size(1)"), "(call s.size 1)");
        assert_eq!(expr("s.len"), "(. s len)");
    }

    /// Rust's struct literal is read as Mente's after its diagnostic, whose fix writes it so;
    /// after a condition, `{` opens the block that follows.
    #[test]
    fn test_a_struct_literal_in_braces_gets_its_fix() {
        let src = "fn f() = Pair { i: 0, j: x + 1 }";
        let (f, errors) = parse(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0129"]);
        let Some(Body::Expr(e)) = f.body else {
            panic!("an expression body");
        };
        assert_eq!(f.tree.render_expr(e), "(new Pair (i 0) (j (+ x 1)))");
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "fn f() = Pair(i: 0, j: x + 1)"
        );
        let multi = "fn f() = Pair {\n  i: 0,\n  j: 1\n}";
        let (_, errors) = parse(multi);
        assert_eq!(
            apply_fix(multi, &errors[0].fixes[0]).unwrap(),
            "fn f() = Pair(i: 0,\n  j: 1)"
        );
        assert!(codes("if c {\n  return\n}").is_empty());
        assert!(codes("while ok {\n  break\n}").is_empty());
        assert_eq!(codes("guard ok { x: 1 }"), ["E0111"]);
        // A name and a block that does not begin `name:` are no struct literal.
        assert_eq!(codes("_ = x { y }"), ["E0112"]);
        // After a struct literal its statement ends at the end of its line, as any other: the
        // `-x` below is a statement of its own, not a subtraction.
        assert_eq!(codes("val p = P { a: 1 }\np.a"), ["E0129"]);
        let (f, _) = parse("fn f() {\n_ = P { a: 1 }\n-x\n}");
        let Some(Body::Block(b)) = f.body else {
            panic!("a block body");
        };
        assert_eq!(f.tree.block(b).stmts.len(), 2);
        // A literal left open ends at its first error.
        assert_eq!(
            self::errors("_ = P { a: 1 ]"),
            [(
                "E0111",
                "expected `,` or the end of the fields, found `]`".to_string()
            )]
        );
        // The lookahead stops at the last token.
        assert_eq!(
            parse("fn f() = P { a")
                .1
                .iter()
                .map(|d| d.code)
                .collect::<Vec<_>>(),
            ["E0111"]
        );
        // A named argument is `name:` and a value, whether a space follows the colon or not;
        // `name::` is no named argument but a path.
        assert_eq!(expr("P(a:b)"), "(new P (a b))");
        assert_eq!(codes("_ = f(x::y)"), ["E0123"]);
    }

    #[test]
    fn test_an_array_length_takes_no_suffix() {
        assert_eq!(codes("var x: [3_i8]i32 = [0; 3]"), ["E0116"]);
        assert_eq!(codes("var x: [300_i8]i32 = [0; 3]"), ["E0116"]);
        assert!(codes(
            "var x: [3]i32 = [0; 3]
_ = x"
        )
        .is_empty());
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
            (
                "unreachable(\"checked above\")",
                "(unreachable \"checked above\")",
            ),
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
            block(
                "_ = f(x)
_ = a + 1
_ == b"
            ),
            "{(discard (call f x)); (discard (+ a 1)); (== _ b)}"
        );
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
        assert_eq!(
            errors("val x = unreachable(1)"),
            vec![(
                "E0111",
                String::from(
                    "`unreachable` takes its reason as a string, as in `unreachable(\"why\")`; found `1`"
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
        let (f, errs) = parse("fn f() {\n  val x = comptime {\n    1 => 2\n  }\n  val z = 3\n}");
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
                    "`a + b` is not a place to assign to; a place is a name, or an element or a field of one such as `a[i]` or `s.f`"
                )
            )]
        );
        // The error ends the statement: an idiom in the value, such as `&`, is not read.
        assert_eq!(
            errors("b + 1 = &mut")
                .iter()
                .map(|(code, _)| *code)
                .collect::<Vec<_>>(),
            ["E0114"]
        );
        assert_eq!(
            errors("f(mut a + b)"),
            vec![(
                "E0114",
                String::from(
                    "after `mut`, an argument is a place, a name or an element or a field of one, not `a + b`"
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
                "a string value comes later in M2; the compiler does not have it yet",
            ),
            (
                "val x = 1.5",
                "a floating-point or fixed-point value comes in M5; M2 does not have it",
            ),
            ("raise E.Bad", "`raise` comes later in M2; the compiler does not have it yet"),
            ("val n = none", "`none` comes later in M2; the compiler does not have it yet"),
            ("val c = catch", "`catch` comes later in M2; the compiler does not have it yet"),
            (
                "val k = comptime f()",
                "`comptime` comes in M4; M2 does not have it",
            ),
            (
                "with Arena.scoped() {\n}",
                "`with` comes in M4; M2 does not have it",
            ),
            (
                "val g = |x| x",
                "a closure comes in M4; M2 does not have it",
            ),
            (
                "parallel (a(), b())",
                "`parallel` comes in M7; M2 does not have it",
            ),
            (
                "parallel_for x in xs {\n}",
                "`parallel_for` comes in M7; M2 does not have it",
            ),
            (
                "for i in 0..4 {\n}",
                "a `for` loop is in no milestone yet; M2 does not have it",
            ),
            (
                "val l = xs[0].count()",
                "a method call comes later in M2; the compiler does not have it yet",
            ),
            (
                "val v = xs[..]",
                "a view `x[..]` comes later in M2; the compiler does not have it yet",
            ),
            (
                "val v = xs[1..3]",
                "a range `x[a..b]` comes later in M2; the compiler does not have it yet",
            ),
            (
                "val v = xs[1..=3]",
                "a range `x[a..b]` comes later in M2; the compiler does not have it yet",
            ),
            // A range inside a call inside an index is an expression's range, not a view's.
            (
                "val v = xs[f(1..3)]",
                "a range in an expression is in no milestone yet; M2 does not have it",
            ),
            // An index's brackets end with it: a range later at the same depth is an expression's.
            (
                "val v = f(g(xs[0]), h(k(1..3)))",
                "a range in an expression is in no milestone yet; M2 does not have it",
            ),
            (
                "val r = f()?",
                "error propagation (`?` and `!`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "val r = f()!",
                "error propagation (`?` and `!`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "val o = a ?? b",
                "an optional value's operator (`??` or `?.`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "val o = a?.b",
                "an optional value's operator (`??` or `?.`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "val y = x |> f()",
                "the pipe `|>` is in no milestone yet; M2 does not have it",
            ),
            (
                "val r = 0..4",
                "a range in an expression is in no milestone yet; M2 does not have it",
            ),
            (
                "val r = 0..=4",
                "a range in an expression is in no milestone yet; M2 does not have it",
            ),
            (
                "pool[mut h] = 1",
                "a `mut` projection `p[mut h]` comes in M3; M2 does not have it",
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
                "a type parameter list comes in M4; M2 does not have it",
            ),
            (
                "fn f(own x: u8) = x",
                "an `own` parameter comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f(in a: u8) = a",
                "an `in` parameter comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f() -> u8 ! E = 1",
                "an error type `T ! E` comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f(d: [4]u8) -> u8 from d = 1",
                "a `from` clause comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f() -> u8 with IO = 1",
                "an effect clause `with` comes in M4; M2 does not have it",
            ),
            (
                "fn f(x: []u8) = 1",
                "a slice type `[]T` comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f(x: ?u8) = 1",
                "an optional type `?T` comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f(x: str) = 1",
                "a string or list type (`str`) comes later in M2; the compiler does not have it yet",
            ),
            (
                "fn f(x: f64) = 1",
                "a floating-point, decimal or saturating type (`f64`) comes in M5; M2 does not have it",
            ),
            (
                "fn f(x: Pool) = 1",
                "an arena, pool or layout type (`Pool`) comes in M3; M2 does not have it",
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

    /// Parses the block body `body`, which holds one idiom, and returns the source and its one
    /// diagnostic.
    fn idiom(body: &str) -> (String, Diagnostic) {
        let src = format!("fn f() {{\n{}\n}}", body);
        let (_, mut errs) = parse(&src);
        assert_eq!(errs.len(), 1, "{}: {:?}", body, errs);
        (src, errs.remove(0))
    }

    #[test]
    fn test_other_languages_idioms_name_the_mente_spelling() {
        // Rewrites that are unique: (body, code, message, the body once the fix is applied).
        let unique = [
            (
                "let x = 1",
                "E0118",
                "Mente writes `val`, not `let`",
                "val x = 1",
            ),
            (
                "let mut y = 2",
                "E0118",
                "Mente writes `var`, not `let mut`",
                "var y = 2",
            ),
            (
                "f(&mut v)",
                "E0119",
                "a call passes a place to change as `mut x`, not `&mut x`",
                "f(mut v)",
            ),
            (
                "val ok = a && b",
                "E0124",
                "Mente writes `and`, not `&&`",
                "val ok = a and b",
            ),
            (
                "val ok = a || b",
                "E0124",
                "Mente writes `or`, not `||`",
                "val ok = a or b",
            ),
            (
                "val ok = !a",
                "E0124",
                "Mente writes `not`, not `!`",
                "val ok = not a",
            ),
            (
                "val ok = !a == b",
                "E0124",
                "Mente writes `not`, not `!`",
                "val ok = (not a) == b",
            ),
            (
                "val y = c ? 1 : 2",
                "E0125",
                "Mente has no `? :`; write `if c { a } else { b }`",
                "val y = if c { 1 } else { 2 }",
            ),
            (
                "val y = x > 0 ? x : 0 - x",
                "E0125",
                "Mente has no `? :`; write `if c { a } else { b }`",
                "val y = if x > 0 { x } else { 0 - x }",
            ),
            (
                "val n = parse::<u8>(s)",
                "E0123",
                "Mente writes type arguments as `f<T>(...)`, without `::`",
                "val n = parse<u8>(s)",
            ),
        ];
        for (body, code, message, after) in unique {
            let (src, d) = idiom(body);
            assert_eq!((d.code, d.message.as_str()), (code, message), "{}", body);
            assert_eq!(
                d.fixes[0].applicability,
                Applicability::MachineApplicable,
                "{}",
                body
            );
            let repaired = apply_fix(&src, &d.fixes[0]).unwrap();
            assert_eq!(repaired, format!("fn f() {{\n{}\n}}", after), "{}", body);
        }
        for after in [
            "val x = 1",
            "var y = 2",
            "f(mut v)",
            "val ok = (not a) == b",
            "val y = if x > 0 { x } else { 0 - x }",
        ] {
            assert!(codes(after).is_empty(), "{}", after);
        }
        // Rewrites that need judgement are suggestions.
        let judged = [
            (
                "f(&v)",
                "E0119",
                "Mente has no references: a parameter is read-only unless it is `mut`, so pass `x`, not `&x`",
                "f(v)",
            ),
            (
                "val r = &mut v",
                "E0119",
                "Mente has no references: a call passes a place to change as `mut x`, and nothing else takes `&mut x`",
                "val r = v",
            ),
            (
                "val n = x.await",
                "E0121",
                "Mente has no `await`: a function that waits declares its effects with `with`, and tasks run at once with `parallel (...)`",
                "val n = x",
            ),
            (
                "val n = await g(1)",
                "E0121",
                "Mente has no `await`: a function that waits declares its effects with `with`, and tasks run at once with `parallel (...)`",
                "val n = g(1)",
            ),
            (
                "val v = r.unwrap()",
                "E0122",
                "Mente has no `.unwrap()`: test the value with `guard`, or give a default with `??` (M2)",
                "val v = r",
            ),
            (
                "val v = r.expect(g(1))",
                "E0122",
                "Mente has no `.expect()`: test the value with `guard`, or give a default with `??` (M2)",
                "val v = r",
            ),
            (
                "val y = x as u8",
                "E0126",
                "Mente has no casts: an integer converts where it lands, checked there, so bind it with the type it needs, as in `val y: u8 = x`",
                "val y = x",
            ),
            (
                "val m = cmp::max(a, b)",
                "E0123",
                "Mente separates the parts of a path with `.`, not `::`",
                "val m = cmp.max(a, b)",
            ),
        ];
        for (body, code, message, after) in judged {
            let (src, d) = idiom(body);
            assert_eq!((d.code, d.message.as_str()), (code, message), "{}", body);
            assert_eq!(
                d.fixes[0].applicability,
                Applicability::Suggestion,
                "{}",
                body
            );
            let repaired = apply_fix(&src, &d.fixes[0]).unwrap();
            assert_eq!(repaired, format!("fn f() {{\n{}\n}}", after), "{}", body);
        }
        // An `async` block gets the code, with no fix.
        let (_, d) = idiom("async {\n  f()\n}");
        assert_eq!((d.code, d.fixes.len()), ("E0121", 0));
        // A `?` with no value after it, or no `:`, is error propagation, a construct of M2.
        assert_eq!(codes("val r = f()?"), vec!["E0109"]);
        assert_eq!(codes("val r = f() ? 1"), vec!["E0109"]);
        // A name, then a name on the next line, is two statements, not a `let`.
        assert_eq!(block("x\ny"), "{x; y}");
        // A `?` before a closing parenthesis is error propagation, not `? :`.
        assert_eq!(codes("val r = g(f()?)"), vec!["E0109"]);
        // The report on a reference covers what was written.
        let (src, d) = idiom("val r = &mut v");
        assert_eq!(&src[d.primary.start..d.primary.end], "&mut");
        // `& &`, apart, is not `&&`.
        assert_eq!(codes("val b = a & &c"), vec!["E0119"]);
        // `&&` and `||` group as `and` and `or` do.
        for (src, want) in [
            ("fn f() = a || b && c", "(or a (and b c))"),
            ("fn f() = a && b && c", "(and (and a b) c)"),
        ] {
            let (f, errs) = parse(src);
            assert_eq!(errs.len(), 2, "{}", src);
            let Some(Body::Expr(e)) = f.body else {
                panic!("no expression body")
            };
            assert_eq!(f.tree.render_expr(e), want, "{}", src);
        }
        // A call the tokens never close is removed to their end.
        let (_, errs) = parse("fn f() = r.unwrap(");
        assert_eq!(
            errs.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec!["E0122"]
        );
        // After an idiom whose meaning is plain, the rest of the statement is still checked.
        assert_eq!(codes("let x = 1 +\n"), vec!["E0118", "E0110"]);
        let (f, errs) = parse("fn f() {\n  let mut i = 0\n  i += 1\n}");
        assert_eq!(errs.len(), 1);
        let Some(Body::Block(b)) = f.body else {
            panic!("no block body")
        };
        assert_eq!(f.tree.render_block(b), "{(var i 0); (+= i 1)}");
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
            depth: 0,
            index_nest: None,
            cond_nest: None,
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

    /// Every shape of nesting stops at the limit with `E0127`, the parser's own recursion and a
    /// long chain the parser reads in a loop alike, and code within the limit parses. Each runs
    /// on a stack of the size `mn` gives its work.
    #[test]
    fn test_code_nested_past_the_limit_is_rejected() {
        let codes = |src: String| {
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn(move || {
                    let m = crate::parse_module(&src).unwrap();
                    crate::parse_bodies(&src, &m)
                        .into_iter()
                        .flat_map(|p| p.errors)
                        .map(|d| d.code)
                        .collect::<Vec<_>>()
                })
                .unwrap()
                .join()
                .unwrap()
        };
        let r = |s: &str, n: usize| s.repeat(n);
        let f = |body: String| format!("module m\npub fn d(x: i32) -> i32 = {}\n", body);
        let past = [
            f(format!("{}x{}", r("(", 300), r(")", 300))),
            f(format!("{}x{}", r("(", 50_000), r(")", 50_000))),
            f(format!("x{}", r(" + 0", 300))),
            f(format!("{}x", r("- ", 300))),
            f(format!(
                "if x > 0 {{ 1 }}{} else {{ 3 }}",
                r(" else if x > 1 { 2 }", 300)
            )),
            f(format!("[{}x{}][0]", r("[", 300), r("]", 300))),
            // Structs being built, nested, and a chain the parser reads in a loop inside them:
            // within the parser's limit each, past it together.
            f(format!("P(a: x{})", r(" + 0", 300))),
            f(format!("{}x{}", r("P(a: ", 300), r(")", 300))),
            f(format!(
                "{}x{}{}",
                r("P(a: ", 120),
                r(" + 0", 200),
                r(")", 120)
            )),
            format!("module m\npub fn d(x: {}i32) -> i32 = 0\n", r("[1]", 300)),
            format!(
                "module m\npub fn d(x: i32) -> i32 {{\n{}{}  x\n}}\n",
                r("  while x > 0 {\n", 300),
                r("  }\n", 300)
            ),
        ];
        for src in past {
            assert_eq!(
                codes(src.clone()),
                ["E0127"],
                "{}",
                &src[..src.len().min(80)]
            );
        }
        for src in [
            f(format!("{}x{}", r("(", 200), r(")", 200))),
            f(format!("x{}", r(" + 0", 200))),
            f(format!("{}x", r("- ", 200))),
        ] {
            assert!(codes(src.clone()).is_empty(), "{}", &src[..80]);
        }
        // The depth goes back down after each construct, so siblings never add up to the limit.
        for stmt in [
            "_ = -x",
            "_ = if x > 0 { 1 } else { 2 }",
            "_ = if x > 0 { 1 } else if x > 1 { 2 } else { 3 }",
            "while x > 0 {\n  }",
        ] {
            let src = format!(
                "module m\npub fn d(x: i32) -> i32 {{\n{}  0\n}}\n",
                format!("  {}\n", stmt).repeat(300)
            );
            assert!(codes(src).is_empty(), "300 of {}", stmt);
        }
        // A struct's field type nested past the limit is reported once, and one within it is
        // read.
        let struct_codes = |src: String| {
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn(move || {
                    let m = crate::parse_module(&src).unwrap();
                    crate::parse_structs(&src, &m)
                        .into_iter()
                        .flat_map(|p| p.errors)
                        .map(|d| d.code)
                        .collect::<Vec<_>>()
                })
                .unwrap()
                .join()
                .unwrap()
        };
        let deep = |n: usize| format!("module m\nstruct S {{\n  a: {}u8\n}}\n", r("[1]", n));
        assert_eq!(struct_codes(deep(300)), ["E0127"]);
        assert!(struct_codes(deep(200)).is_empty());
        // A chain past the limit with another error in it is measured too: `mn dump --tree`
        // recurses over a tree with errors (the fuzzer's finding 12).
        for (src, other) in [
            (f(format!("x{}[0", r("[0]", 300))), "E0111"),
            (f(format!("x{} +", r(" + 1", 300))), "E0110"),
        ] {
            let got = codes(src.clone());
            assert!(
                got.contains(&"E0127") && got.contains(&other),
                "{}: {:?}",
                &src[..80],
                got
            );
        }
    }

    /// An enum declaration, `grammar.md` item 10.
    fn parse_enum_src(src: &str) -> (EnumSyntax, Vec<Diagnostic>) {
        let (tokens, errors) = lex(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let tokens: Vec<Token> = tokens
            .into_iter()
            .filter(|t| t.kind != TokenKind::Eof)
            .collect();
        parse_enum(src, &tokens)
    }

    fn enum_codes(src: &str) -> Vec<&'static str> {
        parse_enum_src(src).1.iter().map(|d| d.code).collect()
    }

    /// An enum's variants stand one per line, each a name and its payload's named fields; a
    /// comma after one, a payload of unnamed types or in braces, and an enum without a variant
    /// are reported, each with its code, and the fixes apply.
    #[test]
    fn test_enum_declarations_take_one_variant_per_line() {
        let (e, errors) = parse_enum_src(
            "enum Shape {\n  /// Nothing.\n  Dot\n  Rect(w: u8[1..=9], h: [2]u8)\n  Line(\n    from: Point,\n    to: Point,\n  )\n}",
        );
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(
            Tree::render_enum(&e),
            "(enum Shape Dot (Rect (w u8[1..=9]) (h [2]u8)) (Line (from Point) (to Point)))"
        );
        assert_eq!(enum_codes("enum E {\n}"), ["E0111"]);
        assert_eq!(enum_codes("enum E {\n  A B\n}"), ["E0112"]);
        assert_eq!(enum_codes("enum E<T> {\n  A\n}"), ["E0109"]);
        let (_, errors) = parse_enum_src("enum E<T> {\n  A\n}");
        assert_eq!(
            errors[0].message,
            "an enum with type parameters comes in M4; M2 does not have it"
        );
        assert_eq!(enum_codes("enum E {\n  A = 1\n}"), ["E0109"]);
        // After a payload of unnamed types, read to its `)`, the next line is the next
        // variant's, so each line reports its own error.
        assert_eq!(
            enum_codes("enum E {\n  A(u8)\n  B(u16)\n  C(u32)\n  D,\n  F,\n}"),
            ["E0130", "E0130", "E0130", "E0128", "E0128"]
        );
        // An attribute above a variant, or before it on its line, is read and left aside.
        let (e, errors) = parse_enum_src("enum E {\n  @deprecated\n  A\n  @since(2) B(n: u8)\n}");
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(Tree::render_enum(&e), "(enum E A (B (n u8)))");
        // A payload in braces whose types are unnamed is told to name its fields.
        let (_, errors) = parse_enum_src("enum E {\n  Rect { u8, u16 }\n}");
        assert!(
            errors[0]
                .message
                .starts_with("a variant's payload names its fields"),
            "{}",
            errors[0].message
        );
        // A payload's type nested too deep is `E0127`, once.
        let deep = format!("{}u8", "[1]".repeat(300));
        let codes_of = |src: String| {
            let mut c = enum_codes(&src);
            c.sort_unstable();
            c
        };
        assert_eq!(
            codes_of(format!("enum E {{\n  A(x: {})\n  B(y: {})\n}}", deep, deep)),
            ["E0127"]
        );
        // A payload holds a field at least: empty parentheses are `E0111`, at the parentheses,
        // whose fix removes them.
        let src = "enum E {\n  A()\n  B\n}";
        let (_, errors) = parse_enum_src(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0111"]);
        assert_eq!(&src[errors[0].primary.start..errors[0].primary.end], "()");
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "enum E {\n  A\n  B\n}"
        );
        assert_eq!(enum_codes("enum E {\n  A(n u8)\n}"), ["E0111"]);
        // A comma after a variant, removed by its fix.
        let src = "enum E {\n  A,\n  B(n: u8),\n}";
        let (_, errors) = parse_enum_src(src);
        assert_eq!(
            errors.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["E0128", "E0128"]
        );
        let once = apply_fix(src, &errors[0].fixes[0]).unwrap();
        assert_eq!(once, "enum E {\n  A\n  B(n: u8),\n}");
        // A payload of unnamed types: one field is `value`, several `value0` and on.
        for (src, fixed) in [
            (
                "enum E {\n  Deposit(i64)\n}",
                "enum E {\n  Deposit(value: i64)\n}",
            ),
            (
                "enum E {\n  Move(i32, [2]u8)\n}",
                "enum E {\n  Move(value0: i32, value1: [2]u8)\n}",
            ),
            (
                "enum E {\n  Rect { w: u8, h: u8 }\n}",
                "enum E {\n  Rect ( w: u8, h: u8 )\n}",
            ),
        ] {
            let (_, errors) = parse_enum_src(src);
            assert_eq!(
                errors.iter().map(|d| d.code).collect::<Vec<_>>(),
                ["E0130"],
                "{}",
                src
            );
            assert_eq!(apply_fix(src, &errors[0].fixes[0]).unwrap(), fixed);
            let (_, again) = parse_enum_src(fixed);
            assert!(again.is_empty(), "{}: {:?}", fixed, again);
        }
    }

    /// `match` takes its arms one per line, each a pattern of M2 and an expression or a block;
    /// a newline after `=>` does not end an arm; `a.b(...)` on a name is a variant built or a
    /// call on a receiver, which name resolution tells apart.
    #[test]
    fn test_match_arms_and_patterns_parse() {
        assert_eq!(
            expr("match x {\n  Shape.Dot => 0\n  Shape.Rect(w: a, h: _) =>\n    a\n  -1 => { 2 }\n  7_u8 => 3\n  true => 4\n  _ => 5\n}"),
            "(match x (=> Shape.Dot 0) (=> (Shape.Rect (w a) (h _)) a) (=> -1 {2}) (=> 7_u8 3) (=> true 4) (=> _ 5))"
        );
        assert_eq!(
            block("match c {\n  true => { x = 1 }\n  false => {\n    x = 2\n  }\n}\ny"),
            "{(match c (=> true {(= x 1)}) (=> false {(= x 2)})); y}"
        );
        // An assignment is a statement, which an arm holds in a block, as its fix writes it.
        let src = "fn f() {\n  match c {\n    true => x += 1\n    false => {}\n  }\n}";
        let (_, errors) = parse(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0111"]);
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "fn f() {\n  match c {\n    true => { x += 1 }\n    false => {}\n  }\n}"
        );
        assert_eq!(
            expr("Shape.Rect(w: 1, h: x)"),
            "(new Shape.Rect (w 1) (h x))"
        );
        assert_eq!(expr("Shape.Dot"), "(. Shape Dot)");
        assert_eq!(
            expr("f(Shape.Rect(1, 2))"),
            "(call f (call Shape.Rect 1 2))"
        );
        // A method call on a language type is a later milestone's, reported where it is
        // written; and an error inside a call's arguments resumes after its `)`, so what
        // follows the call is still read.
        assert_eq!(codes("val xs = List.new()"), ["E0109"]);
        // A `match` the file ends inside is reported as one without its `}`.
        let (_, errors) = parse("fn f() = match c {\n  true => 1");
        assert_eq!(
            errors
                .iter()
                .map(|d| (d.code, d.message.as_str()))
                .collect::<Vec<_>>(),
            [("E0111", "this `match` has no closing `}`")]
        );
        // A call whose arguments the file ends inside, after an error among them, stops at the
        // end of the file.
        let (_, errors) = parse("fn f() = s.f(\"t\" +");
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0109"]);
        // A name followed by one `:` in a pattern is no path written with `::`.
        assert_eq!(codes("_ = match x {\n  v: u8 => 1\n}"), ["E0109"]);
        // After an arm holding an assignment, read whole, the next line is the next arm's, so
        // each arm reports its own error.
        assert_eq!(
            codes("match x {\n  0 => y = 1\n  _ => y = 2\n  1 => y = 3\n}"),
            ["E0111", "E0111", "E0111"]
        );
        // A variant built in braces, as another language's struct literal, is `E0129`, read as
        // its payload in parentheses, which its fix writes.
        let src = "fn f() {\n  val c = E.B { n: 1, m: 2 }\n  _ = c\n}";
        let (_, errors) = parse(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0129"]);
        assert_eq!(
            errors[0].message,
            "Mente builds a variant's payload with its fields in parentheses, as in `E.B(field: value)`, not in braces"
        );
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "fn f() {\n  val c = E.B(n: 1, m: 2)\n  _ = c\n}"
        );
        assert_eq!(
            codes("guard s.f(\"t\") else { return }\nval r = 0..1"),
            ["E0109", "E0109"]
        );
        // A positional binding is read, for the checker to name its field.
        assert_eq!(
            expr("match s {\n  Shape.Rect(a, _) => a\n}"),
            "(match s (=> (Shape.Rect a _) a))"
        );
        // Inside parentheses, a newline still ends an arm.
        assert_eq!(
            expr("(match c {\n  true => 1\n  false => 0\n})"),
            "(match c (=> true 1) (=> false 0))"
        );
        // As an operand, or as a condition, a `match` needs parentheses, which its fix adds.
        let src = "fn f() = 1 + match c {\n  true => 1\n  false => 0\n}";
        let (_, errors) = parse(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0115"]);
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "fn f() = 1 + (match c {\n  true => 1\n  false => 0\n})"
        );
        assert_eq!(codes("while match c {\n  true => 1\n} {\n}"), ["E0115"]);
        assert_eq!(codes("_ = match c {\n}"), ["E0111"]);
        assert_eq!(codes("_ = match c {\n  true => 1"), ["E0111"]);
        assert_eq!(codes("_ = match c {\n  true 1\n}"), ["E0111"]);
        assert_eq!(codes("_ = match c {\n  true => 1 false => 0\n}"), ["E0112"]);
        assert_eq!(codes("_ = match c {\n  - => 1\n}"), ["E0111"]);
        assert_eq!(codes("_ = match c {\n  + => 1\n}"), ["E0111"]);
    }

    /// Each pattern another language has and M2 does not, an arm guard, alternatives, a range, a
    /// nested pattern, a tuple, has its code; a pattern of `?T` is M2's later construct.
    #[test]
    fn test_other_languages_patterns_get_their_codes() {
        for (src, code) in [
            ("_ = match x {\n  0 => 1\n  _ if x > 1 => 2\n}", "E0131"),
            ("_ = match x {\n  0 | 1 => 1\n}", "E0132"),
            ("_ = match x {\n  0..3 => 1\n}", "E0132"),
            ("_ = match x {\n  (a, b) => 1\n}", "E0132"),
            ("_ = match x {\n  [a] => 1\n}", "E0132"),
            ("_ = match x {\n  O.W(inner: I.A) => 1\n}", "E0132"),
            ("_ = match x {\n  O.W(n: 0) => 1\n}", "E0132"),
            ("_ = match x {\n  O.W(I.A) => 1\n}", "E0132"),
            ("_ = match x {\n  none => 1\n}", "E0109"),
            ("_ = match x {\n  v => 1\n}", "E0109"),
            ("_ = match x {\n  E::A => 1\n}", "E0123"),
        ] {
            assert_eq!(codes(src), [code], "{}", src);
        }
        // A comma after an arm, removed by its fix; the arms after it still parse.
        let src = "fn f() = match c {\n  true => 1,\n  false => 0\n}";
        let (f, errors) = parse(src);
        assert_eq!(errors.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0128"]);
        assert_eq!(
            apply_fix(src, &errors[0].fixes[0]).unwrap(),
            "fn f() = match c {\n  true => 1\n  false => 0\n}"
        );
        let Some(Body::Expr(e)) = f.body else {
            panic!("an expression body")
        };
        assert!(matches!(&f.tree.expr(e).kind, ExprKind::Match { arms, .. } if arms.len() == 2));
    }
}

//! `mente-syntax` — `.mn` Holographic Single-File Lexer & Structural Parser.
//!
//! Governed by `spec/holographic-module.md` and `spec/surface-syntax.md`.
//!
//! Parses declarations only. Function and `@example` bodies are kept as flat token lists
//! (`body_tokens`); types, contracts and effect clauses are kept as raw strings. Characters the
//! lexer does not know are skipped without an error.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenKind {
    // Keywords
    Module,
    Import,
    Pub,
    Extern,
    Val,
    Var,
    Mut,
    Own,
    Const,
    Struct,
    BitStruct,
    Enum,
    Trait,
    Type,
    Unit,
    Dyn,
    Fn,
    Return,
    If,
    Else,
    Match,
    Guard,
    While,
    For,
    In,
    Parallel,
    ParallelFor,
    Actor,
    With,
    As,
    Comptime,
    Requires,
    Ensures,
    Invariant,
    Decreases,
    Raise,
    Catch,
    And,
    Or,
    Not,
    Is,
    NoneLit,
    TrueLit,
    FalseLit,

    // Identifiers & Literals
    Ident(String),
    IntLit(i128, String),
    FloatLit(String),
    StrLit(String),
    DocComment(String),

    // Punctuation & Operators
    At,         // @
    LParen,     // (
    RParen,     // )
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    Colon,      // :
    Comma,      // ,
    Dot,        // .
    DotDot,     // ..
    DotDotEq,   // ..=
    Ellipsis,   // ...
    Arrow,      // ->
    FatArrow,   // =>
    PipeGt,     // |>
    Pipe,       // |
    Question,   // ?
    QuestionDot,// ?.
    QuestionQuestion, // ??
    Bang,       // !
    Eq,         // =
    EqEq,       // ==
    NotEq,      // !=
    Lt,         // <
    Le,         // <=
    Gt,         // >
    Ge,         // >=
    Shl,        // <<
    Shr,        // >>
    Plus,       // +
    PlusEq,     // +=
    Minus,      // -
    MinusEq,    // -=
    Star,       // *
    Slash,      // /
    Percent,    // %
    Semi,       // ; (used in array repeat [val; N])
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn tokenize(src: &str) -> Vec<Token> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut tokens = Vec::new();

    while i < len {
        let b = bytes[i];
        if b == b'\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        // Comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let start = i;
            let start_col = col;
            let is_doc = i + 2 < len && bytes[i + 2] == b'/' && (i + 3 >= len || bytes[i + 3] != b'/');
            i += 2;
            if is_doc {
                i += 1;
            }
            let text_start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            if is_doc {
                let doc = src[text_start..i].trim().to_string();
                tokens.push(Token {
                    kind: TokenKind::DocComment(doc),
                    span: Span { start, end: i, line, col: start_col },
                });
            }
            continue;
        }

        let start = i;
        let start_col = col;

        // String literal
        if b == b'"' {
            i += 1;
            col += 1;
            let s_start = i;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    col += 2;
                } else {
                    i += 1;
                    col += 1;
                }
            }
            let content = src[s_start..i].to_string();
            if i < len {
                i += 1;
                col += 1;
            }
            tokens.push(Token {
                kind: TokenKind::StrLit(content),
                span: Span { start, end: i, line, col: start_col },
            });
            continue;
        }

        // Number literal (including unit/fixed-point suffixes like 100.0_meter or 0.5_q16)
        if b.is_ascii_digit() {
            let mut is_float = false;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
                col += 1;
            }
            if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                is_float = true;
                i += 1;
                col += 1;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                    col += 1;
                }
            }
            // Scientific notation e.g. 1e12 or 1e-9
            if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                is_float = true;
                i += 1;
                col += 1;
                if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                    col += 1;
                }
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
            }
            // Optional unit/type suffix starting with alphabetic or _
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
                col += 1;
            }
            let raw = &src[start..i];
            if is_float {
                tokens.push(Token {
                    kind: TokenKind::FloatLit(raw.to_string()),
                    span: Span { start, end: i, line, col: start_col },
                });
            } else {
                let digits_only: String = raw
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '_')
                    .filter(|c| *c != '_')
                    .collect();
                let val = digits_only.parse::<i128>().unwrap_or(0);
                tokens.push(Token {
                    kind: TokenKind::IntLit(val, raw.to_string()),
                    span: Span { start, end: i, line, col: start_col },
                });
            }
            continue;
        }

        // Identifiers & Keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
                col += 1;
            }
            let word = &src[start..i];
            let kind = match word {
                "module" => TokenKind::Module,
                "import" => TokenKind::Import,
                "pub" => TokenKind::Pub,
                "extern" => TokenKind::Extern,
                "val" => TokenKind::Val,
                "var" => TokenKind::Var,
                "mut" => TokenKind::Mut,
                "own" => TokenKind::Own,
                "const" => TokenKind::Const,
                "struct" => TokenKind::Struct,
                "bitstruct" => TokenKind::BitStruct,
                "enum" => TokenKind::Enum,
                "trait" => TokenKind::Trait,
                "type" => TokenKind::Type,
                "unit" => TokenKind::Unit,
                "dyn" => TokenKind::Dyn,
                "fn" => TokenKind::Fn,
                "return" => TokenKind::Return,
                "if" => TokenKind::If,
                "else" => TokenKind::Else,
                "match" => TokenKind::Match,
                "guard" => TokenKind::Guard,
                "while" => TokenKind::While,
                "for" => TokenKind::For,
                "in" => TokenKind::In,
                "parallel" => TokenKind::Parallel,
                "parallel_for" => TokenKind::ParallelFor,
                "actor" => TokenKind::Actor,
                "with" => TokenKind::With,
                "as" => TokenKind::As,
                "comptime" => TokenKind::Comptime,
                "requires" => TokenKind::Requires,
                "ensures" => TokenKind::Ensures,
                "invariant" => TokenKind::Invariant,
                "decreases" => TokenKind::Decreases,
                "raise" => TokenKind::Raise,
                "catch" => TokenKind::Catch,
                "and" => TokenKind::And,
                "or" => TokenKind::Or,
                "not" => TokenKind::Not,
                "is" => TokenKind::Is,
                "none" => TokenKind::NoneLit,
                "true" => TokenKind::TrueLit,
                "false" => TokenKind::FalseLit,
                _ => TokenKind::Ident(word.to_string()),
            };
            tokens.push(Token {
                kind,
                span: Span { start, end: i, line, col: start_col },
            });
            continue;
        }

        // Multi-character operators (matched directly on byte slice to be 100% UTF-8 safe)
        let two: &[u8] = if i + 1 < len { &bytes[i..i + 2] } else { b"" };
        let three: &[u8] = if i + 2 < len { &bytes[i..i + 3] } else { b"" };

        let (kind, advance) = match (three, two, b) {
            (b"..=", _, _) => (TokenKind::DotDotEq, 3),
            (b"...", _, _) => (TokenKind::Ellipsis, 3),
            (_, b"..", _) => (TokenKind::DotDot, 2),
            (_, b"->", _) => (TokenKind::Arrow, 2),
            (_, b"=>", _) => (TokenKind::FatArrow, 2),
            (_, b"|>", _) => (TokenKind::PipeGt, 2),
            (_, b"?.", _) => (TokenKind::QuestionDot, 2),
            (_, b"??", _) => (TokenKind::QuestionQuestion, 2),
            (_, b"==", _) => (TokenKind::EqEq, 2),
            (_, b"!=", _) => (TokenKind::NotEq, 2),
            (_, b"<=", _) => (TokenKind::Le, 2),
            (_, b">=", _) => (TokenKind::Ge, 2),
            (_, b"<<", _) => (TokenKind::Shl, 2),
            (_, b">>", _) => (TokenKind::Shr, 2),
            (_, b"+=", _) => (TokenKind::PlusEq, 2),
            (_, b"-=", _) => (TokenKind::MinusEq, 2),
            (_, _, b'@') => (TokenKind::At, 1),
            (_, _, b'(') => (TokenKind::LParen, 1),
            (_, _, b')') => (TokenKind::RParen, 1),
            (_, _, b'{') => (TokenKind::LBrace, 1),
            (_, _, b'}') => (TokenKind::RBrace, 1),
            (_, _, b'[') => (TokenKind::LBracket, 1),
            (_, _, b']') => (TokenKind::RBracket, 1),
            (_, _, b':') => (TokenKind::Colon, 1),
            (_, _, b',') => (TokenKind::Comma, 1),
            (_, _, b'.') => (TokenKind::Dot, 1),
            (_, _, b'|') => (TokenKind::Pipe, 1),
            (_, _, b'?') => (TokenKind::Question, 1),
            (_, _, b'!') => (TokenKind::Bang, 1),
            (_, _, b'=') => (TokenKind::Eq, 1),
            (_, _, b'<') => (TokenKind::Lt, 1),
            (_, _, b'>') => (TokenKind::Gt, 1),
            (_, _, b'+') => (TokenKind::Plus, 1),
            (_, _, b'-') => (TokenKind::Minus, 1),
            (_, _, b'*') => (TokenKind::Star, 1),
            (_, _, b'/') => (TokenKind::Slash, 1),
            (_, _, b'%') => (TokenKind::Percent, 1),
            (_, _, b';') => (TokenKind::Semi, 1),
            _ => {
                let ch_len = src[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                i += ch_len;
                col += 1;
                continue;
            }
        };

        i += advance;
        col += advance as u32;
        tokens.push(Token {
            kind,
            span: Span { start, end: i, line, col: start_col },
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span { start: len, end: len, line, col },
    });
    tokens
}

// ============================================================================
// Parsed Structural Representation of a `.mn` Holographic File
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub args: String,
}

impl fmt::Display for Attr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "@{}", self.name)
        } else {
            write!(f, "@{}({})", self.name, self.args)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode {
    ReadOnly,
    Mut,
    Own,
    RegionIn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDecl {
    pub mode: ParamMode,
    pub name: String,
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructFieldDecl {
    pub attrs: Vec<Attr>,
    pub byte_range: Option<(usize, usize)>,
    pub is_pub: bool,
    pub name: String,
    pub ty: String,
    pub default_expr: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub generics: String,
    pub fields: Vec<StructFieldDecl>,
    pub invariants: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitFieldDecl {
    pub bit_range: (usize, usize),
    pub is_pub: bool,
    pub name: String,
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitStructDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub backing_type: String,
    pub fields: Vec<BitFieldDecl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub generics: String,
    pub params: Vec<ParamDecl>,
    pub return_ty: Option<String>,
    pub error_ty: Option<String>,
    /// Parameters a returned view may borrow from, named by `from p, q` after the return type.
    pub provenance: Vec<String>,
    pub effects: Vec<String>,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
    pub decreases: Vec<String>,
    pub body_tokens: Vec<TokenKind>,
    pub is_expr_body: bool,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitDecl {
    pub raw_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDecl {
    pub is_pub: bool,
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitDecl {
    pub is_pub: bool,
    pub name: String,
    pub methods_sig: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleDecl {
    pub title: String,
    pub body_tokens: Vec<TokenKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemDecl {
    Unit(UnitDecl),
    Struct(StructDecl),
    BitStruct(BitStructDecl),
    Enum(EnumDecl),
    Trait(TraitDecl),
    Fn(FnDecl),
    Example(ExampleDecl),
}

#[derive(Clone, Debug)]
pub struct ModuleFile {
    pub module_path: String,
    pub items: Vec<ItemDecl>,
    pub total_token_count: usize,
}

/// Parses a `.mn` source string into a structured `ModuleFile`.
pub fn parse_module(src: &str) -> Result<ModuleFile, String> {
    let tokens = tokenize(src);
    let total_token_count = tokens.len().saturating_sub(1);
    let mut p = Parser { src, tokens, pos: 0 };
    p.parse_file(total_token_count)
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos.min(self.tokens.len() - 1)].span
    }

    fn next(&mut self) -> Token {
        let tok = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn parse_file(&mut self, total_token_count: usize) -> Result<ModuleFile, String> {
        let mut module_path = String::from("main");
        let mut pending_docs = Vec::new();
        while let TokenKind::DocComment(d) = self.peek().clone() {
            pending_docs.push(d);
            self.next();
        }
        if *self.peek() == TokenKind::Module {
            self.next();
            module_path = self.parse_dotted_ident()?;
        }

        let mut items = Vec::new();
        while *self.peek() != TokenKind::Eof {
            let mut docs = std::mem::take(&mut pending_docs);
            while let TokenKind::DocComment(d) = self.peek().clone() {
                docs.push(d);
                self.next();
            }
            if *self.peek() == TokenKind::Eof {
                break;
            }
            let attrs = self.parse_attrs()?;
            if attrs.len() == 1 && attrs[0].name == "example" {
                let title = attrs[0].args.trim_matches('"').to_string();
                let body_tokens = self.consume_balanced_braces()?;
                items.push(ItemDecl::Example(ExampleDecl { title, body_tokens }));
                continue;
            }
            let is_pub = if *self.peek() == TokenKind::Pub {
                self.next();
                true
            } else {
                false
            };

            match self.peek() {
                TokenKind::Unit => {
                    self.next();
                    let raw_text = self.consume_line_text();
                    items.push(ItemDecl::Unit(UnitDecl { raw_text }));
                }
                TokenKind::Struct => {
                    items.push(ItemDecl::Struct(self.parse_struct(docs, attrs, is_pub)?));
                }
                TokenKind::BitStruct => {
                    items.push(ItemDecl::BitStruct(self.parse_bitstruct(docs, attrs, is_pub)?));
                }
                TokenKind::Enum => {
                    items.push(ItemDecl::Enum(self.parse_enum(is_pub)?));
                }
                TokenKind::Trait => {
                    items.push(ItemDecl::Trait(self.parse_trait(is_pub)?));
                }
                TokenKind::Fn => {
                    items.push(ItemDecl::Fn(self.parse_fn(docs, attrs, is_pub)?));
                }
                _ => {
                    // Advance gracefully on other top-level constructs
                    self.next();
                }
            }
        }

        Ok(ModuleFile {
            module_path,
            items,
            total_token_count,
        })
    }

    fn parse_dotted_ident(&mut self) -> Result<String, String> {
        let mut out = self.expect_ident()?;
        while *self.peek() == TokenKind::Dot {
            self.next();
            out.push('.');
            out.push_str(&self.expect_ident()?);
        }
        Ok(out)
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.next().kind {
            TokenKind::Ident(s) => Ok(s),
            other => Err(format!("Expected identifier at line {}, got {:?}", self.span().line, other)),
        }
    }

    fn parse_attrs(&mut self) -> Result<Vec<Attr>, String> {
        let mut attrs = Vec::new();
        while *self.peek() == TokenKind::At {
            self.next();
            let name = self.expect_ident()?;
            let mut args = String::new();
            if *self.peek() == TokenKind::LParen {
                args = self.consume_balanced_parens_str()?;
            }
            attrs.push(Attr { name, args });
        }
        Ok(attrs)
    }

    fn consume_balanced_parens_str(&mut self) -> Result<String, String> {
        let open = self.next();
        let start_byte = open.span.end;
        let mut depth = 1usize;
        let mut end_byte = start_byte;
        while *self.peek() != TokenKind::Eof {
            let tok = self.next();
            match tok.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        end_byte = tok.span.start;
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(self.src[start_byte..end_byte].trim().to_string())
    }

    fn consume_balanced_angles_str(&mut self) -> Result<String, String> {
        let open = self.next();
        let start_byte = open.span.start;
        let mut depth = 1usize;
        let mut end_byte = open.span.end;
        while *self.peek() != TokenKind::Eof {
            let tok = self.next();
            match tok.kind {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        end_byte = tok.span.end;
                        break;
                    }
                }
                TokenKind::Shr => {
                    depth = depth.saturating_sub(2);
                    if depth == 0 {
                        end_byte = tok.span.end;
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(self.src[start_byte..end_byte].trim().to_string())
    }

    fn consume_balanced_braces(&mut self) -> Result<Vec<TokenKind>, String> {
        let mut collected = Vec::new();
        if *self.peek() != TokenKind::LBrace {
            return Err(format!("Expected '{{' at line {}", self.span().line));
        }
        self.next();
        let mut depth = 1usize;
        while *self.peek() != TokenKind::Eof && depth > 0 {
            let tok = self.next();
            match tok.kind {
                TokenKind::LBrace => {
                    depth += 1;
                    collected.push(TokenKind::LBrace);
                }
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth > 0 {
                        collected.push(TokenKind::RBrace);
                    }
                }
                other => collected.push(other),
            }
        }
        Ok(collected)
    }

    /// Consumes the `}` that closes a declaration, or reports the declaration as unclosed.
    fn expect_closing_brace(&mut self, what: &str, name: &str) -> Result<Token, String> {
        if *self.peek() == TokenKind::RBrace {
            Ok(self.next())
        } else {
            Err(format!("Line {}: `{} {}` has no closing `}}`", self.span().line, what, name))
        }
    }

    fn consume_line_text(&mut self) -> String {
        let cur_line = self.span().line;
        let start_byte = self.span().start;
        let mut end_byte = start_byte;
        while *self.peek() != TokenKind::Eof && self.span().line == cur_line {
            end_byte = self.next().span.end;
        }
        self.src[start_byte..end_byte].trim().to_string()
    }

    fn parse_struct(&mut self, docs: Vec<String>, attrs: Vec<Attr>, is_pub: bool) -> Result<StructDecl, String> {
        self.next(); // consume 'struct'
        let name = self.expect_ident()?;
        let generics = if *self.peek() == TokenKind::Lt {
            self.consume_balanced_angles_str()?
        } else {
            String::new()
        };
        if *self.peek() != TokenKind::LBrace {
            return Err(format!("Expected '{{' after struct {}", name));
        }
        self.next();

        let mut fields = Vec::new();
        let mut invariants = Vec::new();

        while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
            if let TokenKind::DocComment(_) = self.peek() {
                self.next();
                continue;
            }
            if *self.peek() == TokenKind::Invariant {
                self.next();
                if *self.peek() == TokenKind::LBrace {
                    let open = self.next();
                    let start_b = open.span.end;
                    let mut depth = 1usize;
                    let mut end_b = start_b;
                    while *self.peek() != TokenKind::Eof && depth > 0 {
                        let t = self.next();
                        if t.kind == TokenKind::LBrace {
                            depth += 1;
                        } else if t.kind == TokenKind::RBrace {
                            depth -= 1;
                            if depth == 0 {
                                end_b = t.span.start;
                            }
                        }
                    }
                    for line in self.src[start_b..end_b].lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with("//") {
                            invariants.push(trimmed.to_string());
                        }
                    }
                }
                continue;
            }

            let f_attrs = self.parse_attrs()?;
            let mut byte_range = None;
            for a in &f_attrs {
                if a.name == "bytes" {
                    byte_range = parse_range_arg(&a.args);
                }
            }
            let f_pub = if *self.peek() == TokenKind::Pub {
                self.next();
                true
            } else {
                false
            };
            let f_name = self.expect_ident()?;
            if *self.peek() == TokenKind::Colon {
                self.next();
            }
            let (ty, default_expr) = self.consume_field_type_and_default();
            fields.push(StructFieldDecl {
                attrs: f_attrs,
                byte_range,
                is_pub: f_pub,
                name: f_name,
                ty,
                default_expr,
            });
        }
        self.expect_closing_brace("struct", &name)?;
        Ok(StructDecl {
            docs,
            attrs,
            is_pub,
            name,
            generics,
            fields,
            invariants,
        })
    }

    fn consume_field_type_and_default(&mut self) -> (String, Option<String>) {
        let line = self.span().line;
        let start = self.span().start;
        let mut eq_byte = None;
        let mut end = start;
        let mut bracket_depth = 0i32;
        while *self.peek() != TokenKind::Eof && self.span().line == line && *self.peek() != TokenKind::RBrace {
            let tok = self.next();
            match tok.kind {
                TokenKind::LBracket | TokenKind::Lt | TokenKind::LParen => bracket_depth += 1,
                TokenKind::RBracket | TokenKind::Gt | TokenKind::RParen => bracket_depth -= 1,
                // In a type, `>>` closes two type-argument lists.
                TokenKind::Shr => bracket_depth -= 2,
                TokenKind::Eq if bracket_depth <= 0 && eq_byte.is_none() => {
                    eq_byte = Some((tok.span.start, tok.span.end));
                }
                _ => {}
            }
            end = tok.span.end;
        }
        if let Some((eq_s, eq_e)) = eq_byte {
            let ty = self.src[start..eq_s].trim().trim_end_matches(',').to_string();
            let def = self.src[eq_e..end].trim().trim_end_matches(',').to_string();
            (ty, Some(def))
        } else {
            (self.src[start..end].trim().trim_end_matches(',').to_string(), None)
        }
    }

    fn parse_bitstruct(&mut self, docs: Vec<String>, attrs: Vec<Attr>, is_pub: bool) -> Result<BitStructDecl, String> {
        self.next(); // consume 'bitstruct'
        let name = self.expect_ident()?;
        if *self.peek() == TokenKind::Colon {
            self.next();
        }
        let backing_type = self.expect_ident()?;
        if *self.peek() == TokenKind::LBrace {
            self.next();
        }
        let mut fields = Vec::new();
        while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
            let f_attrs = self.parse_attrs()?;
            let mut bit_range = (0, 0);
            for a in &f_attrs {
                if a.name == "bits" {
                    if let Some(r) = parse_range_arg(&a.args) {
                        bit_range = r;
                    }
                }
            }
            let f_pub = if *self.peek() == TokenKind::Pub {
                self.next();
                true
            } else {
                false
            };
            let f_name = self.expect_ident()?;
            if *self.peek() == TokenKind::Colon {
                self.next();
            }
            let (ty, _) = self.consume_field_type_and_default();
            fields.push(BitFieldDecl {
                bit_range,
                is_pub: f_pub,
                name: f_name,
                ty,
            });
        }
        self.expect_closing_brace("bitstruct", &name)?;
        Ok(BitStructDecl {
            docs,
            attrs,
            is_pub,
            name,
            backing_type,
            fields,
        })
    }

    fn parse_enum(&mut self, is_pub: bool) -> Result<EnumDecl, String> {
        self.next(); // consume 'enum'
        let name = self.expect_ident()?;
        let mut variants = Vec::new();
        if *self.peek() == TokenKind::LBrace {
            let open = self.next();
            let start_b = open.span.end;
            let mut end_b = start_b;
            while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
                end_b = self.next().span.end;
            }
            end_b = self.expect_closing_brace("enum", &name)?.span.start;
            for v in self.src[start_b..end_b].split(',') {
                let t = v.trim();
                if !t.is_empty() {
                    variants.push(t.to_string());
                }
            }
        }
        Ok(EnumDecl { is_pub, name, variants })
    }

    fn parse_trait(&mut self, is_pub: bool) -> Result<TraitDecl, String> {
        self.next(); // consume 'trait'
        let name = self.expect_ident()?;
        let mut methods_sig = Vec::new();
        if *self.peek() == TokenKind::LBrace {
            let open = self.next();
            let start_b = open.span.end;
            let mut end_b = start_b;
            while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
                end_b = self.next().span.end;
            }
            end_b = self.expect_closing_brace("trait", &name)?.span.start;
            for line in self.src[start_b..end_b].lines() {
                let t = line.trim();
                if !t.is_empty() {
                    methods_sig.push(t.to_string());
                }
            }
        }
        Ok(TraitDecl {
            is_pub,
            name,
            methods_sig,
        })
    }

    fn parse_fn(&mut self, docs: Vec<String>, attrs: Vec<Attr>, is_pub: bool) -> Result<FnDecl, String> {
        let fn_tok = self.next(); // consume 'fn'
        let line = fn_tok.span.line;
        let name = self.expect_ident()?;
        let generics = if *self.peek() == TokenKind::Lt {
            self.consume_balanced_angles_str()?
        } else {
            String::new()
        };

        let params_raw = if *self.peek() == TokenKind::LParen {
            self.consume_balanced_parens_str()?
        } else {
            String::new()
        };
        let params = parse_param_list(&params_raw);

        let mut return_ty = None;
        let mut error_ty = None;
        if *self.peek() == TokenKind::Arrow {
            self.next();
            let (ret, err) = self.parse_return_and_error_sig();
            return_ty = Some(ret);
            error_ty = err;
        }

        // `from` is contextual: it is a keyword only right after a return type.
        let mut provenance = Vec::new();
        if matches!(self.peek(), TokenKind::Ident(w) if w == "from") {
            self.next();
            loop {
                if let TokenKind::Ident(param) = self.peek().clone() {
                    provenance.push(param);
                    self.next();
                    if *self.peek() == TokenKind::Comma {
                        self.next();
                        continue;
                    }
                }
                break;
            }
            if provenance.is_empty() {
                return Err(format!("Line {}: `from` in the signature of `{}` names no parameter", line, name));
            }
        }

        let mut effects = Vec::new();
        if *self.peek() == TokenKind::With {
            self.next();
            loop {
                if let TokenKind::Ident(eff) = self.peek().clone() {
                    effects.push(eff);
                    self.next();
                    if *self.peek() == TokenKind::Comma {
                        self.next();
                        continue;
                    }
                }
                break;
            }
        }

        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        let mut decreases = Vec::new();

        loop {
            match self.peek() {
                TokenKind::Requires => {
                    self.next();
                    requires.push(self.consume_contract_expr());
                }
                TokenKind::Ensures => {
                    self.next();
                    ensures.push(self.consume_contract_expr());
                }
                TokenKind::Decreases => {
                    self.next();
                    decreases.push(self.consume_contract_expr());
                }
                _ => break,
            }
        }

        let mut body_tokens = Vec::new();
        let mut is_expr_body = false;
        if *self.peek() == TokenKind::Eq {
            self.next();
            is_expr_body = true;
            let expr_line = self.span().line;
            while *self.peek() != TokenKind::Eof && self.span().line == expr_line {
                body_tokens.push(self.next().kind);
            }
        } else if *self.peek() == TokenKind::LBrace {
            body_tokens = self.consume_balanced_braces()?;
        }

        Ok(FnDecl {
            docs,
            attrs,
            is_pub,
            name,
            generics,
            params,
            return_ty,
            error_ty,
            provenance,
            effects,
            requires,
            ensures,
            decreases,
            body_tokens,
            is_expr_body,
            line,
        })
    }

    fn parse_return_and_error_sig(&mut self) -> (String, Option<String>) {
        let start = self.span().start;
        let mut end = start;
        let mut bang_pos = None;
        let mut angle_depth = 0i32;
        let mut bracket_depth = 0i32;

        while *self.peek() != TokenKind::Eof {
            match self.peek() {
                TokenKind::With
                | TokenKind::Requires
                | TokenKind::Ensures
                | TokenKind::Decreases
                | TokenKind::LBrace
                | TokenKind::Eq
                | TokenKind::Pub
                | TokenKind::Fn
                | TokenKind::At
                    if angle_depth == 0 && bracket_depth == 0 =>
                {
                    break;
                }
                TokenKind::Ident(w) if w == "from" && angle_depth == 0 && bracket_depth == 0 => break,
                _ => {
                    let tok = self.next();
                    match tok.kind {
                        TokenKind::Lt => angle_depth += 1,
                        TokenKind::Gt => angle_depth -= 1,
                        // `List<List<u8>>` closes two type-argument lists with one `>>` token.
                        TokenKind::Shr => angle_depth -= 2,
                        TokenKind::LBracket => bracket_depth += 1,
                        TokenKind::RBracket => bracket_depth -= 1,
                        TokenKind::Bang if angle_depth == 0 && bracket_depth == 0 => {
                            bang_pos = Some((tok.span.start, tok.span.end));
                        }
                        _ => {}
                    }
                    end = tok.span.end;
                }
            }
        }

        if let Some((bs, be)) = bang_pos {
            let ret = self.src[start..bs].trim().to_string();
            let err = self.src[be..end].trim().to_string();
            (ret, Some(err))
        } else {
            (self.src[start..end].trim().to_string(), None)
        }
    }

    fn consume_contract_expr(&mut self) -> String {
        let line = self.span().line;
        let start = self.span().start;
        let mut end = start;
        while *self.peek() != TokenKind::Eof
            && self.span().line == line
            && *self.peek() != TokenKind::LBrace
            && *self.peek() != TokenKind::Eq
        {
            end = self.next().span.end;
        }
        self.src[start..end].trim().to_string()
    }
}

fn parse_range_arg(s: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = s.split("..").collect();
    if parts.len() == 2 {
        let start = parts[0].trim().parse::<usize>().ok()?;
        let end = parts[1].trim().parse::<usize>().ok()?;
        Some((start, end))
    } else {
        None
    }
}

fn parse_param_list(raw: &str) -> Vec<ParamDecl> {
    let mut out = Vec::new();
    for part in split_top_level_commas(raw) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(colon_idx) = p.find(':') {
            let left = p[..colon_idx].trim();
            let ty = p[colon_idx + 1..].trim().to_string();
            let (mode, name) = if let Some(rest) = left.strip_prefix("mut ") {
                (ParamMode::Mut, rest.trim().to_string())
            } else if let Some(rest) = left.strip_prefix("own ") {
                (ParamMode::Own, rest.trim().to_string())
            } else if let Some(rest) = left.strip_prefix("in ") {
                (ParamMode::RegionIn, rest.trim().to_string())
            } else {
                (ParamMode::ReadOnly, left.to_string())
            };
            out.push(ParamDecl { mode, name, ty });
        }
    }
    out
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut res = Vec::new();
    let mut depth = 0i32;
    let mut last = 0usize;
    let mut prev = ' ';
    for (idx, ch) in s.char_indices() {
        let after = prev;
        prev = ch;
        match ch {
            '<' | '[' | '(' => depth += 1,
            // The `>` of a function type's `->` closes nothing.
            '>' if after == '-' => {}
            '>' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                res.push(&s[last..idx]);
                last = idx + 1;
            }
            _ => {}
        }
    }
    if last <= s.len() {
        res.push(&s[last..]);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_and_parser_on_basic_mn() {
        let code = r#"
module demo.core
unit sec, ms = 0.001 * sec

@branchless
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64
  requires lo <= hi
= x.max(lo).min(hi)
"#;
        let m = parse_module(code).expect("should parse valid .mn");
        assert_eq!(m.module_path, "demo.core");
        assert_eq!(m.items.len(), 2);
    }

    #[test]
    fn test_from_clause_names_return_provenance() {
        let code = r#"
module demo.views
pub fn verify(token: str, key: str) -> str ! AuthError from token with Clock
"#;
        let m = parse_module(code).expect("should parse a from clause");
        let ItemDecl::Fn(f) = &m.items[0] else { panic!("expected a fn") };
        assert_eq!(f.return_ty.as_deref(), Some("str"));
        assert_eq!(f.error_ty.as_deref(), Some("AuthError"));
        assert_eq!(f.provenance, vec!["token".to_string()]);
        assert_eq!(f.effects, vec!["Clock".to_string()]);
    }

    fn only_fn(code: &str) -> FnDecl {
        let m = parse_module(code).expect("should parse");
        match m.items.as_slice() {
            [ItemDecl::Fn(f)] => f.clone(),
            other => panic!("expected one fn, got {:?}", other),
        }
    }

    #[test]
    fn test_struct_fields_defaults_and_invariants_are_parsed() {
        let code = r#"
module demo.shapes
pub struct Window<T> {
  pub lo: u32 = 0
  hi: u32
  index: Map<str, List<u32>> = Map.empty()
  invariant {
    lo <= hi
    // a comment is not a clause
    hi < 1024
  }
}
"#;
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else { panic!("expected a struct") };
        assert_eq!((s.name.as_str(), s.generics.as_str(), s.is_pub), ("Window", "<T>", true));
        let fields: Vec<_> = s.fields.iter().map(|f| (f.is_pub, f.name.as_str(), f.ty.as_str(), f.default_expr.as_deref())).collect();
        assert_eq!(
            fields,
            vec![
                (true, "lo", "u32", Some("0")),
                (false, "hi", "u32", None),
                (false, "index", "Map<str, List<u32>>", Some("Map.empty()")),
            ]
        );
        assert_eq!(s.invariants, vec!["lo <= hi".to_string(), "hi < 1024".to_string()]);
    }

    #[test]
    fn test_enum_variants_are_parsed() {
        let code = "module demo.net\npub enum Packet { Ack, Data(bytes: []u8), Close }\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Enum(e) = &m.items[0] else { panic!("expected an enum") };
        assert_eq!((e.name.as_str(), e.is_pub), ("Packet", true));
        assert_eq!(e.variants, vec!["Ack", "Data(bytes: []u8)", "Close"]);
    }

    #[test]
    fn test_with_clause_lists_effects() {
        let f = only_fn("module demo.io\npub fn fetch(url: str) -> str ! NetError with Net, Clock, IO\n");
        assert_eq!(f.effects, vec!["Net", "Clock", "IO"]);
        let pure = only_fn("module demo.io\npub fn add(a: i32, b: i32) -> i32 = a + b\n");
        assert!(pure.effects.is_empty());
    }

    #[test]
    fn test_unit_declarations_keep_their_text() {
        let code = "module demo.units\nunit sec, ms = 0.001 * sec\nunit newton = kg * meter / (sec * sec)\n";
        let m = parse_module(code).unwrap();
        let texts: Vec<_> = m
            .items
            .iter()
            .map(|i| match i {
                ItemDecl::Unit(u) => u.raw_text.as_str(),
                other => panic!("expected a unit, got {:?}", other),
            })
            .collect();
        assert_eq!(texts, vec!["sec, ms = 0.001 * sec", "newton = kg * meter / (sec * sec)"]);
    }

    #[test]
    fn test_error_type_follows_bang_in_return() {
        let f = only_fn("module demo.err\npub fn decode(buf: []u8) -> Map<str, List<u8>> ! DecodeError\n");
        assert_eq!(f.return_ty.as_deref(), Some("Map<str, List<u8>>"));
        assert_eq!(f.error_ty.as_deref(), Some("DecodeError"));
        let plain = only_fn("module demo.err\npub fn len(buf: []u8) -> usize\n");
        assert_eq!((plain.return_ty.as_deref(), plain.error_ty), (Some("usize"), None));
    }

    #[test]
    fn test_contract_clauses_are_kept_line_by_line() {
        let f = only_fn(
            r#"
module demo.search
pub fn find(items: []i64, lo: usize, hi: usize) -> usize
  requires lo < hi
  requires hi <= items.len
  ensures lo <= result and result < hi
  decreases hi - lo
{
  lo
}
"#,
        );
        assert_eq!(f.requires, vec!["lo < hi", "hi <= items.len"]);
        assert_eq!(f.ensures, vec!["lo <= result and result < hi"]);
        assert_eq!(f.decreases, vec!["hi - lo"]);
    }

    #[test]
    fn test_parameter_modes_are_recorded() {
        let f = only_fn("module demo.modes\npub fn step(p: Particle, mut q: Particle, own r: Buffer, in arena: Arena)\n");
        let modes: Vec<_> = f.params.iter().map(|p| (p.mode, p.name.as_str(), p.ty.as_str())).collect();
        assert_eq!(
            modes,
            vec![
                (ParamMode::ReadOnly, "p", "Particle"),
                (ParamMode::Mut, "q", "Particle"),
                (ParamMode::Own, "r", "Buffer"),
                (ParamMode::RegionIn, "arena", "Arena"),
            ]
        );
    }

    #[test]
    fn test_nested_braces_end_at_their_match() {
        let code = r#"
module demo.blocks
pub fn outer(x: i32) -> i32 {
  if x > 0 { x } else { 0 }
}
pub fn after(y: i32) -> i32 = y
"#;
        let m = parse_module(code).unwrap();
        assert_eq!(m.items.len(), 2);
        let ItemDecl::Fn(outer) = &m.items[0] else { panic!("expected a fn") };
        let opens = outer.body_tokens.iter().filter(|t| **t == TokenKind::LBrace).count();
        let closes = outer.body_tokens.iter().filter(|t| **t == TokenKind::RBrace).count();
        assert_eq!((opens, closes), (2, 2));
        let ItemDecl::Fn(after) = &m.items[1] else { panic!("expected a fn") };
        assert_eq!(after.name, "after");
    }

    #[test]
    fn test_function_headers_return_types_and_expression_bodies() {
        let block = only_fn("module demo.fns\npub fn clamp(x: i32, hi: i32) -> i32 {\n  x\n}\n");
        assert_eq!((block.return_ty.as_deref(), block.is_expr_body), (Some("i32"), false));
        let expr = only_fn("module demo.fns\npub fn square(x: i32) -> i32 = x * x\n");
        assert_eq!((expr.return_ty.as_deref(), expr.is_expr_body), (Some("i32"), true));
        assert_eq!(expr.body_tokens.len(), 3);
        let unit = only_fn("module demo.fns\nfn tick(mut n: u64) {\n  n += 1\n}\n");
        assert_eq!((unit.return_ty, unit.is_pub), (None, false));
    }

    #[test]
    fn test_generic_parameter_lists_are_kept() {
        let code = "module demo.gen\npub struct RingWindow<T, comptime CAP: usize = 64> {\n  len: usize\n}\npub fn largest<T>(a: T, b: T) -> T = a\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else { panic!("expected a struct") };
        assert_eq!(s.generics, "<T, comptime CAP: usize = 64>");
        let ItemDecl::Fn(f) = &m.items[1] else { panic!("expected a fn") };
        assert_eq!(f.generics, "<T>");
    }

    #[test]
    fn test_parameters_split_only_at_top_level_commas() {
        let f = only_fn("module demo.params\npub fn run(m: Map<str, u32>, xs: [4]u8, cb: fn(u8, u8) -> u8, n: u32)\n");
        let params: Vec<_> = f.params.iter().map(|p| (p.name.as_str(), p.ty.as_str())).collect();
        assert_eq!(
            params,
            vec![("m", "Map<str, u32>"), ("xs", "[4]u8"), ("cb", "fn(u8, u8) -> u8"), ("n", "u32")]
        );
    }

    #[test]
    fn test_return_type_scan_respects_type_arguments() {
        // A function type inside type arguments: its `fn` and `!` belong to the argument.
        let f = only_fn("module demo.ret\npub fn handlers() -> List<fn(u8) -> u8 ! ParseError> ! SetupError\n");
        assert_eq!(f.return_ty.as_deref(), Some("List<fn(u8) -> u8 ! ParseError>"));
        assert_eq!(f.error_ty.as_deref(), Some("SetupError"));
        let inner_only = only_fn("module demo.ret\npub fn parsers() -> List<fn(u8) -> u8 ! ParseError>\n");
        assert_eq!(inner_only.return_ty.as_deref(), Some("List<fn(u8) -> u8 ! ParseError>"));
        assert_eq!(inner_only.error_ty, None);
    }

    #[test]
    fn test_field_default_starts_at_a_top_level_equals() {
        let code = "module demo.fields\npub struct Holder {\n  ring: Ring<CAP = 64>\n  m: Map<str, u32> = Map.empty()\n  grid: [4]u8 = [0, 0, 0, 0]\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else { panic!("expected a struct") };
        let fields: Vec<_> = s.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str(), f.default_expr.as_deref())).collect();
        assert_eq!(
            fields,
            vec![
                ("ring", "Ring<CAP = 64>", None),
                ("m", "Map<str, u32>", Some("Map.empty()")),
                ("grid", "[4]u8", Some("[0, 0, 0, 0]")),
            ]
        );
    }

    #[test]
    fn test_generic_lists_close_at_their_match() {
        let code = "module demo.gen\npub struct Nest<T = List<List<u8>>> {\n  a: T\n}\npub struct Pair<A, B<C>> {\n  b: A\n}\n";
        let m = parse_module(code).unwrap();
        let generics: Vec<_> = m
            .items
            .iter()
            .map(|i| match i {
                ItemDecl::Struct(s) => s.generics.as_str(),
                other => panic!("expected a struct, got {:?}", other),
            })
            .collect();
        assert_eq!(generics, vec!["<T = List<List<u8>>>", "<A, B<C>>"]);
        // An unclosed list ends at the end of the file instead of running past it.
        assert!(parse_module("module demo.gen\npub struct Open<T").is_err());
    }

    #[test]
    fn test_unclosed_declarations_are_rejected() {
        for (code, what) in [
            ("module m\npub struct Open {\n  a: u8\n", "`struct Open` has no closing `}`"),
            ("module m\npub bitstruct Flags : u8 {\n  @bits(0..8) pub raw: u8\n", "`bitstruct Flags` has no closing `}`"),
            ("module m\npub enum Side { Left, Right\n", "`enum Side` has no closing `}`"),
            ("module m\npub trait Probe {\n  fn hit(self) -> bool\n", "`trait Probe` has no closing `}`"),
        ] {
            let err = parse_module(code).unwrap_err();
            assert!(err.contains(what), "{}", err);
        }
        // Closed, the same declarations parse, and the next item is read.
        let closed = "module m\npub enum Side { Left, Right }\npub trait Probe {\n  fn hit(self) -> bool\n}\npub fn after() = 0\n";
        assert_eq!(parse_module(closed).unwrap().items.len(), 3);
    }

    #[test]
    fn test_invariant_block_may_hold_braces() {
        let code = "module demo.inv\npub struct Bag {\n  items: []u32\n  invariant {\n    items.all { x => x > 0 }\n    items.len < 64\n  }\n  extra: u8\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else { panic!("expected a struct") };
        assert_eq!(s.invariants, vec!["items.all { x => x > 0 }", "items.len < 64"]);
        let names: Vec<_> = s.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["items", "extra"]);
    }

    #[test]
    fn test_trait_method_signatures_are_kept() {
        let code = "module demo.io\npub trait ByteSink {\n  fn write_u64(mut self, v: u64)\n  fn flush(mut self) ! IoError\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Trait(t) = &m.items[0] else { panic!("expected a trait") };
        assert_eq!((t.name.as_str(), t.is_pub), ("ByteSink", true));
        assert_eq!(t.methods_sig, vec!["fn write_u64(mut self, v: u64)", "fn flush(mut self) ! IoError"]);
    }
}

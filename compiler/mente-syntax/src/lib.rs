//! `mente-syntax` — `.mn` Holographic Single-File Lexer & Structural Parser.
//!
//! Governed by `spec/holographic-module.md` and `spec/surface-syntax.md`.
//!
//! Parses declarations only. Function and `@example` bodies are kept as flat token lists
//! (`body_tokens`); types, contracts and effect clauses are kept as raw strings. The lexer
//! follows `spec/grammar.md`, item 1, and reports a character Mente does not use, an unclosed
//! string, an unknown escape and a malformed integer literal. A parse error is a [`Diagnostic`]
//! (`spec/diagnostics.md`), and declarations, fields and attributes keep their spans so later
//! checks can point at them.

use std::fmt;

pub mod diagnostics;

use diagnostics::{insertion_point, Applicability, Diagnostic};

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
    /// The value, and the literal as written, suffix included.
    IntLit(u128, String),
    FloatLit(String),
    StrLit(String),
    DocComment(String),

    // Punctuation & Operators
    At,               // @
    LParen,           // (
    RParen,           // )
    LBrace,           // {
    RBrace,           // }
    LBracket,         // [
    RBracket,         // ]
    Colon,            // :
    Comma,            // ,
    Dot,              // .
    DotDot,           // ..
    DotDotEq,         // ..=
    Ellipsis,         // ...
    Arrow,            // ->
    FatArrow,         // =>
    PipeGt,           // |>
    Pipe,             // |
    Amp,              // &
    Caret,            // ^
    Tilde,            // ~
    Question,         // ?
    QuestionDot,      // ?.
    QuestionQuestion, // ??
    Bang,             // !
    Eq,               // =
    EqEq,             // ==
    NotEq,            // !=
    Lt,               // <
    Le,               // <=
    Gt,               // >
    Ge,               // >=
    Shl,              // <<
    Shr,              // >>
    Plus,             // +
    PlusEq,           // +=
    Minus,            // -
    MinusEq,          // -=
    StarEq,           // *=
    SlashEq,          // /=
    PercentEq,        // %=
    AmpEq,            // &=
    PipeEq,           // |=
    CaretEq,          // ^=
    ShlEq,            // <<=
    ShrEq,            // >>=
    Star,             // *
    Slash,            // /
    Percent,          // %
    Semi,             // ; (used in array repeat [val; N])
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// The tokens of `src`, without the lexical errors [`lex`] reports.
pub fn tokenize(src: &str) -> Vec<Token> {
    lex(src).0
}

/// Lexes `src` into tokens, and reports every lexical error on the way (`spec/grammar.md`,
/// item 1): a character Mente does not use (`E0105`), a string literal a newline or the end of
/// the file interrupts (`E0106`), an unknown escape (`E0107`) and a malformed integer literal
/// (`E0108`). Lexing goes on after an error, so each one is reported.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

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
            let is_doc =
                i + 2 < len && bytes[i + 2] == b'/' && (i + 3 >= len || bytes[i + 3] != b'/');
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
                    span: Span {
                        start,
                        end: i,
                        line,
                        col: start_col,
                    },
                });
            }
            continue;
        }

        let start = i;
        let start_col = col;

        // String literal: it ends on its own line, and knows four escapes.
        if b == b'"' {
            i += 1;
            col += 1;
            let s_start = i;
            let mut closed = false;
            while i < len {
                match bytes[i] {
                    b'"' => {
                        closed = true;
                        break;
                    }
                    b'\n' => break,
                    b'\\' => {
                        let escaped = src[i + 1..].chars().next();
                        match escaped {
                            Some('"' | '\\' | 'n' | 't') => {
                                i += 2;
                                col += 2;
                            }
                            Some(c) if c != '\n' => {
                                errors.push(Diagnostic::error(
                                    "E0107",
                                    Span {
                                        start: i,
                                        end: i + 1 + c.len_utf8(),
                                        line,
                                        col,
                                    },
                                    format!(
                                        "`\\{}` is not an escape Mente knows; a string literal may use `\\\"`, `\\\\`, `\\n` and `\\t`",
                                        c
                                    ),
                                ));
                                i += 1 + c.len_utf8();
                                col += 2;
                            }
                            _ => {
                                i += 1;
                                col += 1;
                            }
                        }
                    }
                    _ => {
                        i += src[i..].chars().next().map_or(1, char::len_utf8);
                        col += 1;
                    }
                }
            }
            let content = src[s_start..i].to_string();
            if closed {
                i += 1;
                col += 1;
            } else {
                errors.push(Diagnostic::error(
                    "E0106",
                    Span {
                        start,
                        end: i,
                        line,
                        col: start_col,
                    },
                    "this string literal is not closed on its line; close it with `\"`, and write a line break inside it as `\\n`",
                ));
            }
            tokens.push(Token {
                kind: TokenKind::StrLit(content),
                span: Span {
                    start,
                    end: i,
                    line,
                    col: start_col,
                },
            });
            continue;
        }

        // Number literal: an integer in decimal, `0x` hexadecimal or `0b` binary, with digits
        // grouped by `_` and an optional suffix (`255_u8`); or a decimal float, whose suffix
        // may name a unit or a fixed-point format (`100.0_meter`, `0.5_q16_16`).
        if b.is_ascii_digit() {
            let (radix, prefix) = match (b, bytes.get(i + 1)) {
                (b'0', Some(b'x')) => (16, "0x"),
                (b'0', Some(b'b')) => (2, "0b"),
                _ => (10, ""),
            };
            let is_digit = |c: u8| match radix {
                16 => c.is_ascii_hexdigit(),
                2 => c == b'0' || c == b'1',
                _ => c.is_ascii_digit(),
            };
            let digits_start = i + prefix.len();
            let mut j = digits_start;
            while j < len {
                let separator = bytes[j] == b'_'
                    && j > digits_start
                    && bytes.get(j + 1).is_some_and(|&c| is_digit(c));
                if is_digit(bytes[j]) || separator {
                    j += 1;
                } else {
                    break;
                }
            }
            let is_float = radix == 10
                && ((j + 1 < len && bytes[j] == b'.' && bytes[j + 1].is_ascii_digit())
                    || (j < len && (bytes[j] == b'e' || bytes[j] == b'E')));
            if is_float {
                if bytes[j] == b'.' {
                    j += 1;
                    while j < len
                        && (bytes[j].is_ascii_digit()
                            || (bytes[j] == b'_'
                                && bytes.get(j + 1).is_some_and(u8::is_ascii_digit)))
                    {
                        j += 1;
                    }
                }
                // Scientific notation e.g. 1e12 or 1e-9
                if j < len && (bytes[j] == b'e' || bytes[j] == b'E') {
                    j += 1;
                    if j < len && (bytes[j] == b'+' || bytes[j] == b'-') {
                        j += 1;
                    }
                    while j < len && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                // A float's suffix, after `_`, names a unit or a fixed-point format.
                if j + 1 < len && bytes[j] == b'_' && bytes[j + 1].is_ascii_alphabetic() {
                    j += 2;
                    while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                        j += 1;
                    }
                }
                col += (j - i) as u32;
                i = j;
                tokens.push(Token {
                    kind: TokenKind::FloatLit(src[start..i].to_string()),
                    span: Span {
                        start,
                        end: i,
                        line,
                        col: start_col,
                    },
                });
                continue;
            }

            let digits_end = j;
            // A suffix starts with `_` and a letter that is not a digit of the literal.
            let suffix_starts = j + 1 < len
                && bytes[j] == b'_'
                && bytes[j + 1].is_ascii_alphabetic()
                && !is_digit(bytes[j + 1]);
            let mut problem = None;
            if suffix_starts {
                j += 1;
            } else if j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                problem = Some(j);
            }
            while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let span = Span {
                start,
                end: j,
                line,
                col: start_col,
            };
            let digits: String = src[digits_start..digits_end]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let value = u128::from_str_radix(&digits, radix).ok();
            // Only a hexadecimal or binary literal can lack a digit, or hold one its base lacks.
            let radix_name = if radix == 16 { "hexadecimal" } else { "binary" };
            if digits.is_empty() {
                errors.push(Diagnostic::error(
                    "E0108",
                    span,
                    format!(
                        "`{}` needs at least one {} digit after it",
                        prefix, radix_name
                    ),
                ));
            } else if let Some(at) = problem {
                let c = bytes[at] as char;
                let d = if c == '_' {
                    Diagnostic::error(
                        "E0108",
                        span,
                        "a `_` in a number stands between two digits, or starts its suffix (`255_u8`)",
                    )
                } else if c.is_ascii_digit() || (radix == 16 && c.is_ascii_hexdigit()) {
                    Diagnostic::error(
                        "E0108",
                        span,
                        format!("`{}` is not a {} digit", c, radix_name),
                    )
                } else {
                    let suffix = &src[at..j];
                    Diagnostic::error(
                        "E0108",
                        span,
                        format!(
                            "a literal's suffix starts with `_`: write `{}_{}`",
                            &src[start..at],
                            suffix
                        ),
                    )
                    .with_fix(
                        Applicability::MachineApplicable,
                        format!("insert `_` before `{}`", suffix),
                        insertion_point(Span {
                            start: at,
                            end: at,
                            line,
                            col: start_col + (at - start) as u32,
                        }),
                        "_",
                    )
                };
                errors.push(d);
            } else if value.is_none() {
                errors.push(Diagnostic::error(
                    "E0108",
                    span,
                    "this integer is too large for any integer type; the widest, `u128`, holds up to 2^128 - 1",
                ));
            }
            col += (j - i) as u32;
            i = j;
            tokens.push(Token {
                kind: TokenKind::IntLit(value.unwrap_or(0), src[start..i].to_string()),
                span,
            });
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
                span: Span {
                    start,
                    end: i,
                    line,
                    col: start_col,
                },
            });
            continue;
        }

        // Multi-character operators (matched directly on byte slice to be 100% UTF-8 safe)
        let two: &[u8] = if i + 1 < len { &bytes[i..i + 2] } else { b"" };
        let three: &[u8] = if i + 2 < len { &bytes[i..i + 3] } else { b"" };

        let (kind, advance) = match (three, two, b) {
            (b"..=", _, _) => (TokenKind::DotDotEq, 3),
            (b"...", _, _) => (TokenKind::Ellipsis, 3),
            (b"<<=", _, _) => (TokenKind::ShlEq, 3),
            (b">>=", _, _) => (TokenKind::ShrEq, 3),
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
            (_, b"*=", _) => (TokenKind::StarEq, 2),
            (_, b"/=", _) => (TokenKind::SlashEq, 2),
            (_, b"%=", _) => (TokenKind::PercentEq, 2),
            (_, b"&=", _) => (TokenKind::AmpEq, 2),
            (_, b"|=", _) => (TokenKind::PipeEq, 2),
            (_, b"^=", _) => (TokenKind::CaretEq, 2),
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
            (_, _, b'&') => (TokenKind::Amp, 1),
            (_, _, b'^') => (TokenKind::Caret, 1),
            (_, _, b'~') => (TokenKind::Tilde, 1),
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
                let c = src[i..].chars().next().unwrap_or('?');
                let why = match c {
                    '\'' => "; Mente writes no lifetimes and has no character literals",
                    '#' => "; an attribute is written `@name`",
                    _ => "",
                };
                errors.push(Diagnostic::error(
                    "E0105",
                    Span {
                        start: i,
                        end: i + c.len_utf8(),
                        line,
                        col,
                    },
                    format!("`{}` is not a character Mente uses{}", c, why),
                ));
                i += c.len_utf8();
                col += 1;
                continue;
            }
        };

        i += advance;
        col += advance as u32;
        tokens.push(Token {
            kind,
            span: Span {
                start,
                end: i,
                line,
                col: start_col,
            },
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: len,
            end: len,
            line,
            col,
        },
    });
    (tokens, errors)
}

// ============================================================================
// Parsed Structural Representation of a `.mn` Holographic File
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub args: String,
    /// From `@` to the end of the arguments.
    pub span: Span,
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
    /// From the field's first attribute, or its name, to the end of its type or default.
    pub span: Span,
    pub name_span: Span,
    pub ty_span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
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
    pub span: Span,
    pub name_span: Span,
    pub ty_span: Span,
    /// The `@bits(...)` attribute, when there is one.
    pub bits_span: Option<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitStructDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
    pub backing_type: String,
    pub backing_span: Span,
    pub fields: Vec<BitFieldDecl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDecl {
    pub docs: Vec<String>,
    pub attrs: Vec<Attr>,
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
    pub generics: String,
    pub params: Vec<ParamDecl>,
    pub return_ty: Option<String>,
    pub error_ty: Option<String>,
    /// Parameters a returned view may borrow from, named by `from p, q` after the return type.
    pub provenance: Vec<String>,
    /// The span of each name in `provenance`, in the same order.
    pub provenance_spans: Vec<Span>,
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
pub fn parse_module(src: &str) -> Result<ModuleFile, Diagnostic> {
    let (tokens, lex_errors) = lex(src);
    if let Some(first) = lex_errors.into_iter().next() {
        return Err(first);
    }
    let total_token_count = tokens.len().saturating_sub(1);
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
        last_end: 0,
    };
    p.parse_file(total_token_count)
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    /// Where the last token taken ends, so a construct's span can close there.
    last_end: usize,
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
        self.last_end = tok.span.end;
        tok
    }

    /// From the start of `start` to the end of the last token taken.
    fn span_from(&self, start: Span) -> Span {
        Span {
            end: self.last_end.max(start.end),
            ..start
        }
    }

    /// The token about to be read, as an error message names it.
    fn found(&self) -> String {
        match self.peek() {
            TokenKind::Eof => String::from("the end of the file"),
            _ => {
                let sp = self.span();
                format!("`{}`", &self.src[sp.start..sp.end])
            }
        }
    }

    fn parse_file(&mut self, total_token_count: usize) -> Result<ModuleFile, Diagnostic> {
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
                    items.push(ItemDecl::BitStruct(
                        self.parse_bitstruct(docs, attrs, is_pub)?,
                    ));
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

    fn parse_dotted_ident(&mut self) -> Result<String, Diagnostic> {
        let mut out = self.expect_ident()?;
        while *self.peek() == TokenKind::Dot {
            self.next();
            out.push('.');
            out.push_str(&self.expect_ident()?);
        }
        Ok(out)
    }

    fn expect_ident(&mut self) -> Result<String, Diagnostic> {
        self.expect_name().map(|(name, _)| name)
    }

    /// A name and its span; anything else is `E0101`.
    fn expect_name(&mut self) -> Result<(String, Span), Diagnostic> {
        if let TokenKind::Ident(name) = self.peek().clone() {
            return Ok((name, self.next().span));
        }
        Err(Diagnostic::error(
            "E0101",
            self.span(),
            format!("expected a name, found {}", self.found()),
        ))
    }

    fn parse_attrs(&mut self) -> Result<Vec<Attr>, Diagnostic> {
        let mut attrs = Vec::new();
        while *self.peek() == TokenKind::At {
            let at = self.next().span;
            let name = self.expect_ident()?;
            let mut args = String::new();
            if *self.peek() == TokenKind::LParen {
                args = self.consume_balanced_parens_str()?;
            }
            attrs.push(Attr {
                name,
                args,
                span: self.span_from(at),
            });
        }
        Ok(attrs)
    }

    fn consume_balanced_parens_str(&mut self) -> Result<String, Diagnostic> {
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

    fn consume_balanced_angles_str(&mut self) -> Result<String, Diagnostic> {
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

    fn consume_balanced_braces(&mut self) -> Result<Vec<TokenKind>, Diagnostic> {
        let mut collected = Vec::new();
        if *self.peek() != TokenKind::LBrace {
            return Err(Diagnostic::error(
                "E0102",
                self.span(),
                format!("expected `{{`, found {}", self.found()),
            ));
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
    fn expect_closing_brace(
        &mut self,
        what: &str,
        name: &str,
        name_span: Span,
    ) -> Result<Token, Diagnostic> {
        if *self.peek() == TokenKind::RBrace {
            return Ok(self.next());
        }
        Err(Diagnostic::error(
            "E0103",
            self.span(),
            format!(
                "`{} {}` has no closing `}}`; found {}",
                what,
                name,
                self.found()
            ),
        )
        .with_related(name_span, format!("`{} {}` opens here", what, name))
        .with_fix(
            Applicability::Suggestion,
            format!("close `{} {}` here", what, name),
            insertion_point(self.span()),
            "}\n",
        ))
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

    fn parse_struct(
        &mut self,
        docs: Vec<String>,
        attrs: Vec<Attr>,
        is_pub: bool,
    ) -> Result<StructDecl, Diagnostic> {
        self.next(); // consume 'struct'
        let (name, name_span) = self.expect_name()?;
        let generics = if *self.peek() == TokenKind::Lt {
            self.consume_balanced_angles_str()?
        } else {
            String::new()
        };
        if *self.peek() != TokenKind::LBrace {
            return Err(Diagnostic::error(
                "E0102",
                self.span(),
                format!(
                    "expected `{{` after `struct {}`, found {}",
                    name,
                    self.found()
                ),
            ));
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

            let f_start = self.span();
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
            let (f_name, name_span) = self.expect_name()?;
            if *self.peek() == TokenKind::Colon {
                self.next();
            }
            let (ty, default_expr, ty_span) = self.consume_field_type_and_default();
            fields.push(StructFieldDecl {
                attrs: f_attrs,
                byte_range,
                is_pub: f_pub,
                name: f_name,
                ty,
                default_expr,
                span: self.span_from(f_start),
                name_span,
                ty_span,
            });
        }
        self.expect_closing_brace("struct", &name, name_span)?;
        Ok(StructDecl {
            docs,
            attrs,
            is_pub,
            name,
            name_span,
            generics,
            fields,
            invariants,
        })
    }

    fn consume_field_type_and_default(&mut self) -> (String, Option<String>, Span) {
        let first = self.span();
        let line = first.line;
        let start = first.start;
        let mut eq_byte = None;
        let mut end = start;
        let mut bracket_depth = 0i32;
        while *self.peek() != TokenKind::Eof
            && self.span().line == line
            && *self.peek() != TokenKind::RBrace
        {
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
            let ty = self.src[start..eq_s]
                .trim()
                .trim_end_matches(',')
                .to_string();
            let def = self.src[eq_e..end].trim().trim_end_matches(',').to_string();
            let ty_span = Span {
                end: start + ty.len(),
                ..first
            };
            (ty, Some(def), ty_span)
        } else {
            let ty = self.src[start..end]
                .trim()
                .trim_end_matches(',')
                .to_string();
            let ty_span = Span {
                end: start + ty.len(),
                ..first
            };
            (ty, None, ty_span)
        }
    }

    fn parse_bitstruct(
        &mut self,
        docs: Vec<String>,
        attrs: Vec<Attr>,
        is_pub: bool,
    ) -> Result<BitStructDecl, Diagnostic> {
        self.next(); // consume 'bitstruct'
        let (name, name_span) = self.expect_name()?;
        if *self.peek() == TokenKind::Colon {
            self.next();
        }
        let (backing_type, backing_span) = self.expect_name()?;
        if *self.peek() == TokenKind::LBrace {
            self.next();
        }
        let mut fields = Vec::new();
        while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
            let f_start = self.span();
            let f_attrs = self.parse_attrs()?;
            let mut bit_range = (0, 0);
            let mut bits_span = None;
            for a in &f_attrs {
                if a.name == "bits" {
                    bits_span = Some(a.span);
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
            let (f_name, f_name_span) = self.expect_name()?;
            if *self.peek() == TokenKind::Colon {
                self.next();
            }
            let (ty, _, ty_span) = self.consume_field_type_and_default();
            fields.push(BitFieldDecl {
                bit_range,
                is_pub: f_pub,
                name: f_name,
                ty,
                span: self.span_from(f_start),
                name_span: f_name_span,
                ty_span,
                bits_span,
            });
        }
        self.expect_closing_brace("bitstruct", &name, name_span)?;
        Ok(BitStructDecl {
            docs,
            attrs,
            is_pub,
            name,
            name_span,
            backing_type,
            backing_span,
            fields,
        })
    }

    fn parse_enum(&mut self, is_pub: bool) -> Result<EnumDecl, Diagnostic> {
        self.next(); // consume 'enum'
        let (name, name_span) = self.expect_name()?;
        let mut variants = Vec::new();
        if *self.peek() == TokenKind::LBrace {
            let open = self.next();
            let start_b = open.span.end;
            while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
                self.next();
            }
            let end_b = self
                .expect_closing_brace("enum", &name, name_span)?
                .span
                .start;
            for v in self.src[start_b..end_b].split(',') {
                let t = v.trim();
                if !t.is_empty() {
                    variants.push(t.to_string());
                }
            }
        }
        Ok(EnumDecl {
            is_pub,
            name,
            variants,
        })
    }

    fn parse_trait(&mut self, is_pub: bool) -> Result<TraitDecl, Diagnostic> {
        self.next(); // consume 'trait'
        let (name, name_span) = self.expect_name()?;
        let mut methods_sig = Vec::new();
        if *self.peek() == TokenKind::LBrace {
            let open = self.next();
            let start_b = open.span.end;
            while *self.peek() != TokenKind::RBrace && *self.peek() != TokenKind::Eof {
                self.next();
            }
            let end_b = self
                .expect_closing_brace("trait", &name, name_span)?
                .span
                .start;
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

    fn parse_fn(
        &mut self,
        docs: Vec<String>,
        attrs: Vec<Attr>,
        is_pub: bool,
    ) -> Result<FnDecl, Diagnostic> {
        let fn_tok = self.next(); // consume 'fn'
        let line = fn_tok.span.line;
        let (name, name_span) = self.expect_name()?;
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
        let mut provenance_spans = Vec::new();
        if matches!(self.peek(), TokenKind::Ident(w) if w == "from") {
            let from_span = self.next().span;
            loop {
                if let TokenKind::Ident(param) = self.peek().clone() {
                    provenance.push(param);
                    provenance_spans.push(self.next().span);
                    if *self.peek() == TokenKind::Comma {
                        self.next();
                        continue;
                    }
                }
                break;
            }
            if provenance.is_empty() {
                return Err(Diagnostic::error(
                    "E0104",
                    from_span,
                    format!(
                        "`from` in the signature of `{}` names no parameter; found {}",
                        name,
                        self.found()
                    ),
                ));
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
            name_span,
            generics,
            params,
            return_ty,
            error_ty,
            provenance,
            provenance_spans,
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
                TokenKind::Ident(w) if w == "from" && angle_depth == 0 && bracket_depth == 0 => {
                    break
                }
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

    fn doc_comments(src: &str) -> Vec<String> {
        tokenize(src)
            .into_iter()
            .filter_map(|t| match t.kind {
                TokenKind::DocComment(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (tokens, errors) = lex(src);
        assert!(errors.is_empty(), "{:?}", errors);
        tokens.into_iter().map(|t| t.kind).collect()
    }

    /// The codes and messages of every lexical error in `src`.
    fn lex_errors(src: &str) -> Vec<(&'static str, String)> {
        lex(src)
            .1
            .into_iter()
            .map(|d| (d.code, d.message))
            .collect()
    }

    #[test]
    fn test_lexer_reads_every_m1_literal_and_operator() {
        // Integers in decimal, hexadecimal and binary, grouped by `_`, with a suffix after `_`.
        assert_eq!(
            kinds("1_000_000 0xFF_u8 0xab_cd 0b1010_0101 255_u8 0"),
            vec![
                TokenKind::IntLit(1_000_000, "1_000_000".into()),
                TokenKind::IntLit(255, "0xFF_u8".into()),
                TokenKind::IntLit(0xabcd, "0xab_cd".into()),
                TokenKind::IntLit(0b1010_0101, "0b1010_0101".into()),
                TokenKind::IntLit(255, "255_u8".into()),
                TokenKind::IntLit(0, "0".into()),
                TokenKind::Eof,
            ]
        );
        // The widest value, `u128::MAX`, is a literal; floats keep their unit suffixes.
        let max = u128::MAX.to_string();
        assert_eq!(kinds(&max)[0], TokenKind::IntLit(u128::MAX, max.clone()));
        assert_eq!(
            kinds("1.5 100.0_meter 1e12 1.5e-3 2E+4 0.75_q16_16 1_000.000_5"),
            vec![
                TokenKind::FloatLit("1.5".into()),
                TokenKind::FloatLit("100.0_meter".into()),
                TokenKind::FloatLit("1e12".into()),
                TokenKind::FloatLit("1.5e-3".into()),
                TokenKind::FloatLit("2E+4".into()),
                TokenKind::FloatLit("0.75_q16_16".into()),
                TokenKind::FloatLit("1_000.000_5".into()),
                TokenKind::Eof,
            ]
        );
        // A dot after an integer, with no digit after it, is a dot.
        assert_eq!(
            kinds("a.len 3.len 1."),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Dot,
                TokenKind::Ident("len".into()),
                TokenKind::IntLit(3, "3".into()),
                TokenKind::Dot,
                TokenKind::Ident("len".into()),
                TokenKind::IntLit(1, "1".into()),
                TokenKind::Dot,
                TokenKind::Eof,
            ]
        );
        // Every operator of grammar.md, item 1, that the declaration parser did not need.
        assert_eq!(
            kinds("& ^ ~ *= /= %= &= |= ^= <<= >>= << >> | ;"),
            vec![
                TokenKind::Amp,
                TokenKind::Caret,
                TokenKind::Tilde,
                TokenKind::StarEq,
                TokenKind::SlashEq,
                TokenKind::PercentEq,
                TokenKind::AmpEq,
                TokenKind::PipeEq,
                TokenKind::CaretEq,
                TokenKind::ShlEq,
                TokenKind::ShrEq,
                TokenKind::Shl,
                TokenKind::Shr,
                TokenKind::Pipe,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
        // A string keeps its four escapes as written.
        assert_eq!(
            kinds(r#""a\"b\\c\n\t""#)[0],
            TokenKind::StrLit(r#"a\"b\\c\n\t"#.into())
        );
        // Columns count characters, so a token after a literal starts where it appears.
        let cols: Vec<u32> = lex(r#"0xFF_u8 x 1.5 y "\n" z "é" w"#)
            .0
            .iter()
            .map(|t| t.span.col)
            .collect();
        assert_eq!(cols, vec![1, 9, 11, 15, 17, 22, 24, 28, 29]);
        let (tokens, _) = lex("255_u8 x");
        assert_eq!((tokens[0].span.start, tokens[0].span.end), (0, 6));
    }

    #[test]
    fn test_lexical_errors_are_diagnostics_and_lexing_goes_on() {
        // A character Mente does not use, with a word on why for the ones agents bring along.
        assert_eq!(
            lex_errors("x # y"),
            vec![(
                "E0105",
                "`#` is not a character Mente uses; an attribute is written `@name`".to_string()
            )]
        );
        assert!(lex_errors("fn f<'a>()")[0].1.contains("no lifetimes"));
        assert_eq!(lex_errors("é")[0].1, "`é` is not a character Mente uses");
        let (tokens, _) = lex("é x");
        assert_eq!(tokens[0].span.col, 3);

        // A string ends on its line; lexing goes on with the next one.
        let (tokens, errors) = lex("\"open\nx");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "E0106");
        assert_eq!(tokens[1].kind, TokenKind::Ident("x".into()));
        assert_eq!(lex_errors("\"a\\\nb\"")[0].0, "E0106");
        let (tokens, errors) = lex("\"\\");
        assert_eq!(errors[0].code, "E0106");
        assert_eq!(tokens[1].span.col, 3);

        // Four escapes and no others.
        assert_eq!(
            lex_errors(r#""\q""#),
            vec![(
                "E0107",
                r#"`\q` is not an escape Mente knows; a string literal may use `\"`, `\\`, `\n` and `\t`"#
                    .to_string()
            )]
        );
        let (tokens, _) = lex(r#""\q" z"#);
        assert_eq!(tokens[1].span.col, 6);

        // Malformed integers.
        let malformed = [
            ("0x", "`0x` needs at least one hexadecimal digit after it"),
            ("0x_FF", "`0x` needs at least one hexadecimal digit after it"),
            ("0b", "`0b` needs at least one binary digit after it"),
            ("0b102", "`2` is not a binary digit"),
            ("1__0", "a `_` in a number stands between two digits, or starts its suffix (`255_u8`)"),
            ("7_", "a `_` in a number stands between two digits, or starts its suffix (`255_u8`)"),
            ("0b1e", "a literal's suffix starts with `_`: write `0b1_e`"),
            (
                "340282366920938463463374607431768211456",
                "this integer is too large for any integer type; the widest, `u128`, holds up to 2^128 - 1",
            ),
        ];
        for (src, message) in malformed {
            assert_eq!(
                lex_errors(src),
                vec![("E0108", message.to_string())],
                "{}",
                src
            );
        }
        // A suffix written without `_` gets a fix that inserts it.
        let (_, errors) = lex("val t = 5ms");
        assert_eq!(
            errors[0].message,
            "a literal's suffix starts with `_`: write `5_ms`"
        );
        assert_eq!(
            errors[0].fixes[0].applicability,
            Applicability::MachineApplicable
        );
        let fixed = diagnostics::apply_fix("val t = 5ms", &errors[0].fixes[0]).unwrap();
        assert_eq!(fixed, "val t = 5_ms");
        assert!(lex(&fixed).1.is_empty());

        // Every error is reported, and parse_module stops at the first.
        assert_eq!(
            lex_errors("# \"\\q\" 0b2 $")
                .iter()
                .map(|e| e.0)
                .collect::<Vec<_>>(),
            vec!["E0105", "E0107", "E0108", "E0105"]
        );
        assert_eq!(
            parse_module("module m\npub fn f() -> u8 = 0b2\n# ")
                .unwrap_err()
                .code,
            "E0108"
        );
    }

    #[test]
    fn test_doc_comments_are_exactly_three_slashes() {
        // `///` opens a doc comment; `//` and `////` open plain comments, which leave no token.
        let src = "module m\n/// the doc\n// plain\n//// banner\nfn f() = 0\n///";
        assert_eq!(doc_comments(src), vec!["the doc", ""]);
        // A comment that ends the file, with or without a third slash, does not read past it.
        assert_eq!(doc_comments("x //"), Vec::<String>::new());
        assert_eq!(doc_comments("x ///"), vec![""]);
        assert_eq!(doc_comments("x ////"), Vec::<String>::new());
        // The code before a comment stays code.
        let kinds: Vec<_> = tokenize("val a = 1 // note\n")
            .into_iter()
            .map(|t| t.kind)
            .collect();
        assert!(
            kinds.contains(&TokenKind::Val)
                && !kinds.iter().any(|k| matches!(k, TokenKind::DocComment(_)))
        );
    }

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
        let ItemDecl::Fn(f) = &m.items[0] else {
            panic!("expected a fn")
        };
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
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        assert_eq!(
            (s.name.as_str(), s.generics.as_str(), s.is_pub),
            ("Window", "<T>", true)
        );
        let fields: Vec<_> = s
            .fields
            .iter()
            .map(|f| {
                (
                    f.is_pub,
                    f.name.as_str(),
                    f.ty.as_str(),
                    f.default_expr.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            fields,
            vec![
                (true, "lo", "u32", Some("0")),
                (false, "hi", "u32", None),
                (false, "index", "Map<str, List<u32>>", Some("Map.empty()")),
            ]
        );
        assert_eq!(
            s.invariants,
            vec!["lo <= hi".to_string(), "hi < 1024".to_string()]
        );
    }

    #[test]
    fn test_enum_variants_are_parsed() {
        let code = "module demo.net\npub enum Packet { Ack, Data(bytes: []u8), Close }\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Enum(e) = &m.items[0] else {
            panic!("expected an enum")
        };
        assert_eq!((e.name.as_str(), e.is_pub), ("Packet", true));
        assert_eq!(e.variants, vec!["Ack", "Data(bytes: []u8)", "Close"]);
    }

    #[test]
    fn test_with_clause_lists_effects() {
        let f = only_fn(
            "module demo.io\npub fn fetch(url: str) -> str ! NetError with Net, Clock, IO\n",
        );
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
        assert_eq!(
            texts,
            vec!["sec, ms = 0.001 * sec", "newton = kg * meter / (sec * sec)"]
        );
    }

    #[test]
    fn test_error_type_follows_bang_in_return() {
        let f = only_fn(
            "module demo.err\npub fn decode(buf: []u8) -> Map<str, List<u8>> ! DecodeError\n",
        );
        assert_eq!(f.return_ty.as_deref(), Some("Map<str, List<u8>>"));
        assert_eq!(f.error_ty.as_deref(), Some("DecodeError"));
        let plain = only_fn("module demo.err\npub fn len(buf: []u8) -> usize\n");
        assert_eq!(
            (plain.return_ty.as_deref(), plain.error_ty),
            (Some("usize"), None)
        );
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
        let modes: Vec<_> = f
            .params
            .iter()
            .map(|p| (p.mode, p.name.as_str(), p.ty.as_str()))
            .collect();
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
        let ItemDecl::Fn(outer) = &m.items[0] else {
            panic!("expected a fn")
        };
        let opens = outer
            .body_tokens
            .iter()
            .filter(|t| **t == TokenKind::LBrace)
            .count();
        let closes = outer
            .body_tokens
            .iter()
            .filter(|t| **t == TokenKind::RBrace)
            .count();
        assert_eq!((opens, closes), (2, 2));
        let ItemDecl::Fn(after) = &m.items[1] else {
            panic!("expected a fn")
        };
        assert_eq!(after.name, "after");
    }

    #[test]
    fn test_function_headers_return_types_and_expression_bodies() {
        let block = only_fn("module demo.fns\npub fn clamp(x: i32, hi: i32) -> i32 {\n  x\n}\n");
        assert_eq!(
            (block.return_ty.as_deref(), block.is_expr_body),
            (Some("i32"), false)
        );
        let expr = only_fn("module demo.fns\npub fn square(x: i32) -> i32 = x * x\n");
        assert_eq!(
            (expr.return_ty.as_deref(), expr.is_expr_body),
            (Some("i32"), true)
        );
        assert_eq!(expr.body_tokens.len(), 3);
        let unit = only_fn("module demo.fns\nfn tick(mut n: u64) {\n  n += 1\n}\n");
        assert_eq!((unit.return_ty, unit.is_pub), (None, false));
    }

    #[test]
    fn test_generic_parameter_lists_are_kept() {
        let code = "module demo.gen\npub struct RingWindow<T, comptime CAP: usize = 64> {\n  len: usize\n}\npub fn largest<T>(a: T, b: T) -> T = a\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        assert_eq!(s.generics, "<T, comptime CAP: usize = 64>");
        let ItemDecl::Fn(f) = &m.items[1] else {
            panic!("expected a fn")
        };
        assert_eq!(f.generics, "<T>");
    }

    #[test]
    fn test_parameters_split_only_at_top_level_commas() {
        let f = only_fn("module demo.params\npub fn run(m: Map<str, u32>, xs: [4]u8, cb: fn(u8, u8) -> u8, n: u32)\n");
        let params: Vec<_> = f
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.ty.as_str()))
            .collect();
        assert_eq!(
            params,
            vec![
                ("m", "Map<str, u32>"),
                ("xs", "[4]u8"),
                ("cb", "fn(u8, u8) -> u8"),
                ("n", "u32")
            ]
        );
    }

    #[test]
    fn test_return_type_scan_respects_type_arguments() {
        // A function type inside type arguments: its `fn` and `!` belong to the argument.
        let f = only_fn(
            "module demo.ret\npub fn handlers() -> List<fn(u8) -> u8 ! ParseError> ! SetupError\n",
        );
        assert_eq!(
            f.return_ty.as_deref(),
            Some("List<fn(u8) -> u8 ! ParseError>")
        );
        assert_eq!(f.error_ty.as_deref(), Some("SetupError"));
        let inner_only =
            only_fn("module demo.ret\npub fn parsers() -> List<fn(u8) -> u8 ! ParseError>\n");
        assert_eq!(
            inner_only.return_ty.as_deref(),
            Some("List<fn(u8) -> u8 ! ParseError>")
        );
        assert_eq!(inner_only.error_ty, None);
    }

    #[test]
    fn test_field_default_starts_at_a_top_level_equals() {
        let code = "module demo.fields\npub struct Holder {\n  ring: Ring<CAP = 64>\n  m: Map<str, u32> = Map.empty()\n  grid: [4]u8 = [0, 0, 0, 0]\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        let fields: Vec<_> = s
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty.as_str(), f.default_expr.as_deref()))
            .collect();
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
    fn test_parse_errors_are_diagnostics_with_spans() {
        let at =
            |code: &str, err: &Diagnostic| code[err.primary.start..err.primary.end].to_string();

        let code = "module m\npub struct 42 {\n}\n";
        let err = parse_module(code).unwrap_err();
        assert_eq!((err.code, at(code, &err).as_str()), ("E0101", "42"));
        assert_eq!(err.message, "expected a name, found `42`");
        let err = parse_module("module").unwrap_err();
        assert_eq!(
            (err.code, err.message.as_str()),
            ("E0101", "expected a name, found the end of the file")
        );

        let code = "module m\npub struct S\n  a: u8\n";
        let err = parse_module(code).unwrap_err();
        assert_eq!((err.code, at(code, &err).as_str()), ("E0102", "a"));
        assert_eq!(err.message, "expected `{` after `struct S`, found `a`");
        let code = "module m\n@example(\"x\") 1\n";
        let err = parse_module(code).unwrap_err();
        assert_eq!((err.code, at(code, &err).as_str()), ("E0102", "1"));

        let code = "module m\npub fn f(a: []u8) -> []u8 from with IO\n";
        let err = parse_module(code).unwrap_err();
        assert_eq!((err.code, at(code, &err).as_str()), ("E0104", "from"));
        assert!(err.message.ends_with("found `with`"), "{}", err.message);
    }

    #[test]
    fn test_a_type_span_covers_every_token_of_the_type() {
        let code = "module m
pub struct S {
  buf: [4]u8
  index: Map<str, u32> = Map.empty()
}
";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        let types: Vec<_> = s
            .fields
            .iter()
            .map(|f| &code[f.ty_span.start..f.ty_span.end])
            .collect();
        assert_eq!(types, vec!["[4]u8", "Map<str, u32>"]);
    }

    #[test]
    fn test_declarations_keep_their_spans() {
        let code = "module m\n@layout(size: 4, align: 4)\npub struct S {\n  @bytes(0..4) pub v: u32\n}\npub bitstruct B : u8 {\n  @bits(0..8) pub raw: u8\n}\npub fn f(a: []u8) -> []u8 from a\n";
        let m = parse_module(code).unwrap();
        let text = |sp: Span| &code[sp.start..sp.end];
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        assert_eq!(text(s.attrs[0].span), "@layout(size: 4, align: 4)");
        assert_eq!(text(s.name_span), "S");
        let f = &s.fields[0];
        assert_eq!(
            (
                text(f.span),
                text(f.name_span),
                text(f.ty_span),
                text(f.attrs[0].span)
            ),
            ("@bytes(0..4) pub v: u32", "v", "u32", "@bytes(0..4)")
        );
        let ItemDecl::BitStruct(b) = &m.items[1] else {
            panic!("expected a bitstruct")
        };
        assert_eq!((text(b.name_span), text(b.backing_span)), ("B", "u8"));
        let bf = &b.fields[0];
        assert_eq!(
            (text(bf.span), text(bf.ty_span), bf.bits_span.map(text)),
            ("@bits(0..8) pub raw: u8", "u8", Some("@bits(0..8)"))
        );
        let ItemDecl::Fn(func) = &m.items[2] else {
            panic!("expected a fn")
        };
        assert_eq!(text(func.name_span), "f");
        assert_eq!(
            func.provenance_spans
                .iter()
                .map(|sp| text(*sp))
                .collect::<Vec<_>>(),
            vec!["a"]
        );
    }

    #[test]
    fn test_unclosed_declarations_are_rejected() {
        for (code, what) in [
            (
                "module m\npub struct Open {\n  a: u8\n",
                "`struct Open` has no closing `}`",
            ),
            (
                "module m\npub bitstruct Flags : u8 {\n  @bits(0..8) pub raw: u8\n",
                "`bitstruct Flags` has no closing `}`",
            ),
            (
                "module m\npub enum Side { Left, Right\n",
                "`enum Side` has no closing `}`",
            ),
            (
                "module m\npub trait Probe {\n  fn hit(self) -> bool\n",
                "`trait Probe` has no closing `}`",
            ),
        ] {
            let err = parse_module(code).unwrap_err();
            assert_eq!(err.code, "E0103");
            assert!(err.message.contains(what), "{:?}", err);
            // The related span is the declaration's name, and the suggested fix closes it.
            let name = what.split(' ').nth(1).unwrap().trim_end_matches('`');
            assert_eq!(
                &code[err.related[0].span.start..err.related[0].span.end],
                name
            );
            let fixed = diagnostics::apply_fix(code, &err.fixes[0]).unwrap();
            assert!(parse_module(&fixed).is_ok(), "{}", fixed);
        }
        // Closed, the same declarations parse, and the next item is read.
        let closed = "module m\npub enum Side { Left, Right }\npub trait Probe {\n  fn hit(self) -> bool\n}\npub fn after() = 0\n";
        assert_eq!(parse_module(closed).unwrap().items.len(), 3);
    }

    #[test]
    fn test_invariant_block_may_hold_braces() {
        let code = "module demo.inv\npub struct Bag {\n  items: []u32\n  invariant {\n    items.all { x => x > 0 }\n    items.len < 64\n  }\n  extra: u8\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Struct(s) = &m.items[0] else {
            panic!("expected a struct")
        };
        assert_eq!(
            s.invariants,
            vec!["items.all { x => x > 0 }", "items.len < 64"]
        );
        let names: Vec<_> = s.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["items", "extra"]);
    }

    #[test]
    fn test_trait_method_signatures_are_kept() {
        let code = "module demo.io\npub trait ByteSink {\n  fn write_u64(mut self, v: u64)\n  fn flush(mut self) ! IoError\n}\n";
        let m = parse_module(code).unwrap();
        let ItemDecl::Trait(t) = &m.items[0] else {
            panic!("expected a trait")
        };
        assert_eq!((t.name.as_str(), t.is_pub), ("ByteSink", true));
        assert_eq!(
            t.methods_sig,
            vec![
                "fn write_u64(mut self, v: u64)",
                "fn flush(mut self) ! IoError"
            ]
        );
    }
}

//! `mente-syntax` — `.mn` Holographic Single-File Lexer & Structural Parser.
//!
//! Governed by `spec/holographic-module.md` and `spec/surface-syntax.md`.
//!
//! The declaration parser keeps function and `@example` bodies as flat token lists
//! (`body_tokens`), and types, contracts and effect clauses as raw strings, for the digests;
//! the body parser ([`body`], over [`tree`]) reads each function with a body under M1's
//! grammar, through [`parse_functions`]. The lexer
//! follows `spec/grammar.md`, item 1, and reports a character Mente does not use, an unclosed
//! string, an unknown escape and a malformed integer literal. A parse error is a [`Diagnostic`]
//! (`spec/diagnostics.md`), and declarations, fields and attributes keep their spans so later
//! checks can point at them.

use std::fmt;

pub mod body;
pub mod diagnostics;
pub mod tree;

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
    Break,
    Continue,
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
    /// A lifetime from Rust, such as `'a`: not Mente, and reported as such.
    Lifetime(String),

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

#[derive(Clone, Debug, PartialEq, Eq)]
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
            let digit_at = |k: usize| bytes.get(k).is_some_and(u8::is_ascii_digit);
            let word_at = |k: usize| {
                bytes
                    .get(k)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            };
            // A fraction is `.` and a digit; an exponent is `e` or `E`, then a digit, or a
            // sign and a digit (`1e12`, `1.5e-3`).
            let fraction_at = |k: usize| bytes.get(k) == Some(&b'.') && digit_at(k + 1);
            let exponent_at = |k: usize| {
                matches!(bytes.get(k), Some(b'e' | b'E'))
                    && (digit_at(k + 1)
                        || (matches!(bytes.get(k + 1), Some(b'+' | b'-')) && digit_at(k + 2)))
            };
            if radix == 10 && (fraction_at(j) || exponent_at(j)) {
                if fraction_at(j) {
                    j += 1;
                    while digit_at(j) || (bytes.get(j) == Some(&b'_') && digit_at(j + 1)) {
                        j += 1;
                    }
                }
                if exponent_at(j) {
                    j += 1;
                    if matches!(bytes.get(j), Some(b'+' | b'-')) {
                        j += 1;
                    }
                    while digit_at(j) {
                        j += 1;
                    }
                }
                // A float's suffix, after `_`, names a unit or a fixed-point format.
                if bytes.get(j) == Some(&b'_')
                    && bytes.get(j + 1).is_some_and(u8::is_ascii_alphabetic)
                {
                    while word_at(j) {
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
            // The digits stop at a `_` only when no digit of the literal follows it, so a `_`
            // and a letter start the suffix.
            let suffix_starts = bytes.get(j) == Some(&b'_')
                && bytes.get(j + 1).is_some_and(u8::is_ascii_alphabetic);
            let problem = if !suffix_starts && word_at(j) {
                Some(j)
            } else {
                None
            };
            while word_at(j) {
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

        // A lifetime from Rust (`'a`), which Mente does not write, is a token of its own, so
        // the parser names the idiom (`E0120`); a character literal (`'a'`) stays an error.
        if b == b'\''
            && bytes
                .get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            let mut j = i + 1;
            while bytes
                .get(j)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                j += 1;
            }
            if bytes.get(j) != Some(&b'\'') {
                tokens.push(Token {
                    kind: TokenKind::Lifetime(src[i + 1..j].to_string()),
                    span: Span {
                        start,
                        end: j,
                        line,
                        col: start_col,
                    },
                });
                col += (j - i) as u32;
                i = j;
                continue;
            }
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
                "break" => TokenKind::Break,
                "continue" => TokenKind::Continue,
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
                    '\'' => "; Mente has no character literals: write the character's code, such as `0x41` for `A`",
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

/// A lifetime from Rust (`E0120`). Removing `<'a>`, or the `'a` of `&'a T`, is suggested; what
/// replaces it, a `from` clause or a parameter mode, needs judgement.
fn lifetime_idiom(tokens: &[Token], at: usize) -> Diagnostic {
    let here = tokens[at].span;
    let d = Diagnostic::error(
        "E0120",
        here,
        "Mente writes no lifetimes: a returned view names its source with `from p`, and a parameter that must outlive the call takes the mode `own` or `in`",
    );
    let prev = at.checked_sub(1).and_then(|p| tokens.get(p));
    let next = tokens.get(at + 1);
    match (prev, next) {
        (Some(p), Some(n)) if p.kind == TokenKind::Lt && n.kind == TokenKind::Gt => {
            let list = Span {
                start: p.span.start,
                end: n.span.end,
                ..here
            };
            d.with_fix(
                Applicability::Suggestion,
                "remove the lifetime list",
                list,
                "",
            )
        }
        (Some(p), Some(n)) if p.kind == TokenKind::Amp => {
            let lifetime = Span {
                end: n.span.start,
                ..here
            };
            d.with_fix(
                Applicability::Suggestion,
                "remove the lifetime",
                lifetime,
                "",
            )
        }
        _ => d,
    }
}

/// Whether a statement goes on past a newline after `last`: a token that cannot end one
/// (`spec/grammar.md`, item 2).
fn continues_statement(last: &TokenKind) -> bool {
    matches!(
        last,
        TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Amp
            | TokenKind::Pipe
            | TokenKind::Caret
            | TokenKind::Shl
            | TokenKind::Shr
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::Eq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::AmpEq
            | TokenKind::PipeEq
            | TokenKind::CaretEq
            | TokenKind::ShlEq
            | TokenKind::ShrEq
            | TokenKind::Comma
            | TokenKind::Dot
            | TokenKind::Arrow
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace
    )
}

/// Parses, under M1's grammar, every function of `module` that has a body (`spec/grammar.md`,
/// items 2 to 8), and reports every error the body parser finds. A function without a body is
/// a declaration, which the declaration checks of its spec file read; M1 does not parse it
/// further.
pub fn parse_functions(src: &str, module: &ModuleFile) -> (Vec<tree::FnSyntax>, Vec<Diagnostic>) {
    let mut functions = Vec::new();
    let mut errors = Vec::new();
    for parsed in parse_bodies(src, module) {
        functions.extend(parsed.syntax);
        errors.extend(parsed.errors);
    }
    (functions, errors)
}

/// One function of a module, as [`parse_bodies`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFn {
    pub name: String,
    /// Its syntax tree, when it has a body.
    pub syntax: Option<tree::FnSyntax>,
    /// What the body parser reported in it.
    pub errors: Vec<Diagnostic>,
}

/// One struct of a module, as [`parse_structs`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedStruct {
    pub name: String,
    pub syntax: tree::StructSyntax,
    /// What the body parser reported in it.
    pub errors: Vec<Diagnostic>,
}

/// Whether a struct is one of M2's calculus (`core-calculus.md`, item 13): every struct but one
/// with `@layout`, whose declaration `morphic-layouts.md` governs and whose values come in M3.
pub fn is_value_struct(s: &StructDecl) -> bool {
    !s.attrs.iter().any(|a| a.name == "layout")
}

/// Every struct of `module` that is one of M2's calculus ([`is_value_struct`]), in order, parsed
/// under M2's grammar (`grammar.md`, item 10).
pub fn parse_structs(src: &str, module: &ModuleFile) -> Vec<ParsedStruct> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            ItemDecl::Struct(s) if is_value_struct(s) && !s.tokens.is_empty() => {
                let (syntax, errors) = body::parse_struct(src, &s.tokens);
                Some(ParsedStruct {
                    name: s.name.clone(),
                    syntax,
                    errors,
                })
            }
            _ => None,
        })
        .collect()
}

/// One enum of a module, as [`parse_enums`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedEnum {
    pub name: String,
    pub syntax: tree::EnumSyntax,
    /// What the body parser reported in it.
    pub errors: Vec<Diagnostic>,
}

/// Every enum of `module`, in order, parsed under M2's grammar (`grammar.md`, item 10).
pub fn parse_enums(src: &str, module: &ModuleFile) -> Vec<ParsedEnum> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            ItemDecl::Enum(e) if !e.tokens.is_empty() => {
                let (syntax, errors) = body::parse_enum(src, &e.tokens);
                Some(ParsedEnum {
                    name: e.name.clone(),
                    syntax,
                    errors,
                })
            }
            _ => None,
        })
        .collect()
}

/// Every function of `module`, in order, with the syntax tree of each one that has a body.
pub fn parse_bodies(src: &str, module: &ModuleFile) -> Vec<ParsedFn> {
    let mut functions = Vec::new();
    for item in &module.items {
        if let ItemDecl::Fn(f) = item {
            let has_body =
                f.is_expr_body || f.tokens.last().is_some_and(|t| t.kind == TokenKind::RBrace);
            let (syntax, errors) = if has_body {
                let (syntax, errors) = body::parse_function(src, &f.tokens);
                (Some(syntax), errors)
            } else {
                (None, Vec::new())
            };
            functions.push(ParsedFn {
                name: f.name.clone(),
                syntax,
                errors,
            });
        }
    }
    functions
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
    /// Every token of the struct, from `struct` to its closing `}`, which the body parser reads
    /// under M2's grammar ([`parse_structs`]).
    pub tokens: Vec<Token>,
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
    /// Every token of the function, from `fn` to the end of its body, which the body parser
    /// reads under M1's grammar ([`parse_functions`]).
    pub tokens: Vec<Token>,
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
    pub name_span: Span,
    /// Each variant's text as written, its payload included.
    pub variants: Vec<String>,
    /// Every token of the enum, from `enum` to its closing `}`, which the body parser reads
    /// under M2's grammar ([`parse_enums`]).
    pub tokens: Vec<Token>,
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
    Fn(Box<FnDecl>),
    Example(ExampleDecl),
}

#[derive(Clone, Debug)]
pub struct ModuleFile {
    pub module_path: String,
    pub items: Vec<ItemDecl>,
    pub total_token_count: usize,
}

/// The version of the canonical dumps, which rises when any dump's form changes.
pub const DUMP_VERSION: u32 = 2;

/// The canonical form of a module's tokens: a header line, then one token per line as
/// `line:col Kind text`, the kind the name of its [`TokenKind`] variant and the text its source
/// spells, with `\` and newlines escaped. Two lexers agree when their dumps are byte-identical.
pub fn dump_tokens(src: &str) -> String {
    let mut out = format!("mente-dump {} tokens\n", DUMP_VERSION);
    for t in tokenize(src)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Eof)
    {
        let debug = format!("{:?}", t.kind);
        let kind = debug.split(['(', ' ', '{']).next().unwrap_or("");
        let text = src
            .get(t.span.start..t.span.end)
            .unwrap_or("")
            .replace('\\', "\\\\")
            .replace('\n', "\\n");
        out.push_str(&format!(
            "{}:{} {} {}\n",
            t.span.line, t.span.col, kind, text
        ));
    }
    out
}

/// The canonical form of a module's syntax trees: a header line, then each struct of M2's
/// calculus as [`tree::Tree::render_struct`] writes it and each function with a body as
/// [`tree::Tree::render_fn`] does, one per line, in the order of the module. A module the
/// declaration parser rejects dumps its error as `error line:col code`.
pub fn dump_tree(src: &str) -> String {
    let mut out = format!("mente-dump {} tree\n", DUMP_VERSION);
    match parse_module(src) {
        Err(d) => out.push_str(&format!(
            "error {}:{} {}\n",
            d.primary.line, d.primary.col, d.code
        )),
        Ok(module) => {
            // A tree that nests too deep is not rendered, since rendering recurses over it.
            let too_deep = |errors: &[Diagnostic]| {
                errors
                    .iter()
                    .find(|d| d.code == "E0127")
                    .map(|d| format!("error {}:{} {}", d.primary.line, d.primary.col, d.code))
            };
            let mut lines: Vec<(usize, String)> = Vec::new();
            for parsed in parse_structs(src, &module) {
                let line = too_deep(&parsed.errors)
                    .unwrap_or_else(|| tree::Tree::render_struct(&parsed.syntax));
                lines.push((parsed.syntax.name_span.start, line));
            }
            for parsed in parse_enums(src, &module) {
                let line = too_deep(&parsed.errors)
                    .unwrap_or_else(|| tree::Tree::render_enum(&parsed.syntax));
                lines.push((parsed.syntax.name_span.start, line));
            }
            for parsed in parse_bodies(src, &module) {
                if let Some(f) = &parsed.syntax {
                    let line = too_deep(&parsed.errors).unwrap_or_else(|| tree::Tree::render_fn(f));
                    lines.push((f.name_span.start, line));
                }
            }
            lines.sort_by_key(|(at, _)| *at);
            for (_, line) in lines {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

/// The words that begin a declaration in a line's first column.
const DECLARES: [&str; 14] = [
    "module",
    "import",
    "pub",
    "extern",
    "struct",
    "bitstruct",
    "enum",
    "trait",
    "type",
    "unit",
    "fn",
    "actor",
    "const",
    "comptime",
];

/// Whether a line outside every bracket begins a top-level item, as `text` starts it: its
/// first column holds a word that begins a declaration, or an `@` or a `/` (an attribute or a
/// comment above an item). [`split_items`] splits a module there, and the declaration parser
/// ends an expression body, a contract or a return type there, so the two read the same items.
fn begins_item(text: &str) -> bool {
    let first = text.chars().next().unwrap_or(' ');
    let word = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    matches!(first, '@' | '/') || (first.is_ascii_alphabetic() && DECLARES.contains(&word))
}

/// Where one top-level item's text starts: its byte offset and its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStart {
    pub offset: usize,
    pub line: u32,
}

/// Splits a module's text into its top-level items, each running from its start to the next
/// one's, for parsing and checking item by item. An item starts at a line outside every bracket
/// whose first column holds a word that begins a declaration, such as `pub`, `fn` or `struct`,
/// or an `@` or a `/`; the attributes, doc comments and comments right above an item belong to
/// it, and the module's first item starts at its first byte. Any other line continues the item
/// above it: the `{`, `=` or `}` of a function, and a line of its body or of its expression
/// body however it is indented.
pub fn split_items(src: &str) -> Vec<ItemStart> {
    let mut starts = vec![ItemStart { offset: 0, line: 1 }];
    // Whether the current item holds anything but attributes and comments yet.
    let mut has_body = false;
    // How many brackets are open at the start of the line.
    let mut depth = 0i64;
    let mut offset = 0;
    for (i, line) in src.split_inclusive('\n').enumerate() {
        let prefix = matches!(line.chars().next(), Some('@' | '/'));
        let starts_item = depth <= 0 && begins_item(line);
        depth += bracket_change(line);
        if starts_item && has_body {
            starts.push(ItemStart {
                offset,
                line: i as u32 + 1,
            });
            has_body = false;
        }
        if starts_item && !prefix {
            has_body = true;
        }
        offset += line.len();
    }
    starts
}

/// How many brackets a line opens, less those it closes, outside its comment and its string
/// literals.
fn bracket_change(line: &str) -> i64 {
    let mut change = 0;
    let mut in_string = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => break,
            '(' | '[' | '{' if !in_string => change += 1,
            ')' | ']' | '}' if !in_string => change -= 1,
            _ => {}
        }
    }
    change
}

/// Parses a `.mn` source string into a structured `ModuleFile`.
pub fn parse_module(src: &str) -> Result<ModuleFile, Diagnostic> {
    let (tokens, lex_errors) = lex(src);
    if let Some(first) = lex_errors.into_iter().next() {
        return Err(first);
    }
    if let Some(at) = tokens
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Lifetime(_)))
    {
        return Err(lifetime_idiom(&tokens, at));
    }
    let total_token_count = tokens.len().saturating_sub(1);
    let ends = item_ends(src, &tokens);
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
        last_end: 0,
        ends,
        item: 0,
    };
    p.parse_file(total_token_count)
}

/// Where each item of [`split_items`] ends among `tokens`: the index of the next item's first
/// token, and of `Eof` for the last. The declaration parser reads each item within its own
/// tokens, so that the whole module's check and a `--watch` session, which parses each item's text
/// alone, read the same items: no scan of one item runs into the next.
fn item_ends(src: &str, tokens: &[Token]) -> Vec<usize> {
    let eof = tokens.len().saturating_sub(1);
    // Both lists ascend, so one pass over the tokens finds every item's first token. `Eof`
    // starts at the text's length, past every item's start, so the pass stops there at the
    // latest.
    let mut at = 0;
    let mut ends: Vec<usize> = split_items(src)
        .iter()
        .skip(1)
        .map(|s| {
            while tokens[at].span.start < s.offset {
                at += 1;
            }
            at
        })
        .collect();
    ends.push(eof);
    ends
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    /// Where the last token taken ends, so a construct's span can close there.
    last_end: usize,
    /// Where each item ends ([`item_ends`]), and which item is read.
    ends: Vec<usize>,
    item: usize,
}

/// What the parser reads at the end of an item, as it does at the end of the file.
static EOF: TokenKind = TokenKind::Eof;

impl<'a> Parser<'a> {
    // The tokens end with `Eof`, and only `next` and `next_item` move `pos`, never past the item's
    // end: so `pos` always names a token, and the item ends at `limit`, which reads as `Eof`.
    fn limit(&self) -> usize {
        self.ends[self.item]
    }

    fn peek(&self) -> &TokenKind {
        if self.pos >= self.limit() {
            &EOF
        } else {
            &self.tokens[self.pos].kind
        }
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// The next token, and past it; at the item's end, `Eof` again.
    fn next(&mut self) -> Token {
        let mut tok = self.tokens[self.pos].clone();
        if self.pos >= self.limit() {
            tok.kind = TokenKind::Eof;
        } else {
            self.pos += 1;
        }
        self.last_end = tok.span.end;
        tok
    }

    /// Moves on to the next item, whether this one's tokens are all read or not; false at the
    /// last.
    fn next_item(&mut self) -> bool {
        if self.item + 1 >= self.ends.len() {
            return false;
        }
        self.pos = self.limit();
        self.item += 1;
        true
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
        match &self.tokens[self.pos].kind {
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
        loop {
            if *self.peek() == TokenKind::Eof {
                if self.next_item() {
                    continue;
                }
                break;
            }
            let mut docs = std::mem::take(&mut pending_docs);
            while let TokenKind::DocComment(d) = self.peek().clone() {
                docs.push(d);
                self.next();
            }
            // A doc comment at the end of an item documents nothing, as it does when the item
            // is parsed alone; the items after it are parsed still.
            if *self.peek() == TokenKind::Eof {
                continue;
            }
            let attrs = self.parse_attrs()?;
            if attrs.len() == 1 && attrs[0].name == "example" {
                let title = attrs[0].args.trim_matches('"').to_string();
                let opened = ("@example", attrs[0].args.as_str(), attrs[0].span);
                let body_tokens = self.consume_balanced_braces(opened)?;
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
                    items.push(ItemDecl::Fn(Box::new(self.parse_fn(docs, attrs, is_pub)?)));
                }
                _ => return Err(self.not_a_declaration()),
            }
        }

        Ok(ModuleFile {
            module_path,
            items,
            total_token_count,
        })
    }

    /// What stands where a declaration belongs: a declaration of a later milestone (`E0109`), or
    /// anything else (`E0111`).
    fn not_a_declaration(&self) -> Diagnostic {
        if matches!(self.peek(), TokenKind::Ident(w) if w == "async") {
            let here = self.span();
            let to_next = Span {
                end: self
                    .tokens
                    .get(self.pos + 1)
                    .map_or(here.end, |t| t.span.start),
                ..here
            };
            return Diagnostic::error(
                "E0121",
                here,
                "Mente has no `async`: a function declares its effects with `with`, and runs tasks at once with `parallel (...)`",
            )
            .with_fix(Applicability::Suggestion, "remove `async`", to_next, "");
        }
        let later = match self.peek() {
            TokenKind::Import => Some(("an `import`", "M6")),
            TokenKind::Extern => Some(("an `extern` block", "M4")),
            TokenKind::Actor => Some(("an `actor`", "M7")),
            _ => None,
        };
        match later {
            Some((what, milestone)) => Diagnostic::error(
                "E0109",
                self.span(),
                crate::diagnostics::later_message(what, Some(milestone)),
            ),
            None => Diagnostic::error(
                "E0111",
                self.span(),
                format!(
                    "expected a declaration, such as `fn`, `struct` or `@example`, found {}",
                    self.found()
                ),
            ),
        }
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
        // A list still open at the end of the file is no list. Where it opens is a related
        // span, never text, so that a position moved with the module stays right.
        if depth > 0 {
            return Err(Diagnostic::error(
                "E0111",
                self.span(),
                format!("expected `)` to close this list, found {}", self.found()),
            )
            .with_related(open.span, "the list opens here"));
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

    /// The tokens of a body in braces, the outer pair left out. A body the file ends inside is
    /// unclosed (`E0103`), as a declaration is: `what name`, opened at `name_span`.
    fn consume_balanced_braces(
        &mut self,
        (what, name, name_span): (&str, &str, Span),
    ) -> Result<Vec<TokenKind>, Diagnostic> {
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
        if depth > 0 {
            self.expect_closing_brace(what, name, name_span)?;
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
        let struct_index = self.pos;
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
            tokens: self.tokens[struct_index..self.pos].to_vec(),
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
        let enum_index = self.pos;
        self.next(); // consume 'enum'
        let (name, name_span) = self.expect_name()?;
        // Type parameters, which the body parser reports with their milestone.
        if *self.peek() == TokenKind::Lt {
            self.consume_balanced_angles_str()?;
        }
        let mut variants = Vec::new();
        if *self.peek() == TokenKind::LBrace {
            self.next();
            // Each variant's tokens: a variant ends at a `,` or at the end of its line, outside
            // its payload's brackets, braces included (`grammar.md`, item 10); an attribute on a
            // line of its own belongs to the variant below it.
            let mut depth = 0i32;
            let mut group: Vec<Token> = Vec::new();
            let mut groups: Vec<Vec<Token>> = Vec::new();
            while !(depth <= 0 && *self.peek() == TokenKind::RBrace)
                && *self.peek() != TokenKind::Eof
            {
                let t = self.next();
                match t.kind {
                    TokenKind::DocComment(_) => continue,
                    TokenKind::Comma if depth <= 0 => {
                        groups.push(std::mem::take(&mut group));
                        continue;
                    }
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                    TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
                    _ => {}
                }
                let new_line = group.last().is_some_and(|l| l.span.line < t.span.line);
                // A token that closes a payload stays with its variant.
                let closes = matches!(
                    t.kind,
                    TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace
                );
                if depth <= 0 && new_line && !closes && !attributes_only(&group) {
                    groups.push(std::mem::take(&mut group));
                }
                group.push(t);
            }
            groups.push(group);
            variants = groups
                .iter()
                .filter(|g| !g.is_empty())
                .map(|g| canonical_variant(self.src, g))
                .collect();
            self.expect_closing_brace("enum", &name, name_span)?;
        }
        Ok(EnumDecl {
            is_pub,
            name,
            name_span,
            variants,
            tokens: self.tokens[enum_index..self.pos].to_vec(),
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
        let fn_index = self.pos;
        let fn_tok = self.next(); // consume 'fn'
        let line = fn_tok.span.line;
        let (name, name_span) = self.expect_name()?;
        let generics = if *self.peek() == TokenKind::Lt {
            self.consume_balanced_angles_str()?
        } else {
            String::new()
        };

        // A function always has a parameter list, empty or not.
        if *self.peek() != TokenKind::LParen {
            return Err(Diagnostic::error(
                "E0111",
                self.span(),
                format!(
                    "expected `(` to open the parameter list of `{}`, found {}",
                    name,
                    self.found()
                ),
            ));
        }
        let params_raw = self.consume_balanced_parens_str()?;
        let params = parse_param_list(&params_raw);

        let mut return_ty = None;
        let mut error_ty = None;
        if *self.peek() == TokenKind::Arrow {
            self.next();
            let (ret, err) = self.parse_return_and_error_sig(&name)?;
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
                    requires.push(self.consume_contract_expr()?);
                }
                TokenKind::Ensures => {
                    self.next();
                    ensures.push(self.consume_contract_expr()?);
                }
                TokenKind::Decreases => {
                    self.next();
                    decreases.push(self.consume_contract_expr()?);
                }
                _ => break,
            }
        }

        let mut body_tokens = Vec::new();
        let mut is_expr_body = false;
        if *self.peek() == TokenKind::Eq {
            self.next();
            is_expr_body = true;
            // The body runs to the end of its line, and on past a newline where
            // `spec/grammar.md`, item 2, says the statement goes on.
            let mut depth = 0i32;
            let mut last_line = self.span().line;
            let mut last = TokenKind::Eq;
            while *self.peek() != TokenKind::Eof
                && (depth > 0
                    || self.span().line == last_line
                    || continues_statement(&last)
                    || matches!(self.peek(), TokenKind::Else | TokenKind::LBrace))
            {
                let tok = self.next();
                match tok.kind {
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                    TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
                    _ => {}
                }
                last_line = tok.span.line;
                body_tokens.push(tok.kind.clone());
                last = tok.kind;
            }
        } else if *self.peek() == TokenKind::LBrace {
            body_tokens = self.consume_balanced_braces(("fn", &name, name_span))?;
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
            tokens: self.tokens[fn_index..self.pos].to_vec(),
            line,
        })
    }

    /// The return type of `name`, and its error type after a `!`, as text. Its brackets and
    /// type-argument lists must close in order (`E0116`): a closer that closes nothing would
    /// otherwise take the scan past the body, and an opener never closed would take it past the
    /// items after it.
    fn parse_return_and_error_sig(
        &mut self,
        name: &str,
    ) -> Result<(String, Option<String>), Diagnostic> {
        let start = self.span().start;
        let mut end = start;
        let mut bang_pos = None;
        // The `[` and `<` open where the scan is, innermost last.
        let mut open: Vec<(&str, Span)> = Vec::new();
        let malformed = |span: Span, what: String| {
            Diagnostic::error(
                "E0116",
                span,
                format!("the return type of `{}` is malformed: {}", name, what),
            )
        };

        while *self.peek() != TokenKind::Eof {
            match self.peek() {
                // A type argument may hold a function type, or a named argument such as
                // `Ring<CAP = 64>`: only outside every bracket do these end the type.
                TokenKind::With
                | TokenKind::Requires
                | TokenKind::Ensures
                | TokenKind::Decreases
                | TokenKind::LBrace
                | TokenKind::Eq
                | TokenKind::Pub
                | TokenKind::Fn
                | TokenKind::At
                    if open.is_empty() =>
                {
                    break;
                }
                TokenKind::Ident(w) if w == "from" && open.is_empty() => break,
                _ => {
                    let tok = self.next();
                    // `List<List<u8>>` closes two type-argument lists with one `>>` token.
                    let closes: &[&str] = match tok.kind {
                        TokenKind::Lt => {
                            open.push(("<", tok.span));
                            &[]
                        }
                        TokenKind::LBracket => {
                            open.push(("[", tok.span));
                            &[]
                        }
                        TokenKind::Gt => &["<"],
                        TokenKind::Shr => &["<", "<"],
                        TokenKind::RBracket => &["["],
                        TokenKind::Bang if open.is_empty() => {
                            bang_pos = Some((tok.span.start, tok.span.end));
                            &[]
                        }
                        _ => &[],
                    };
                    for opener in closes {
                        if open.pop().map(|(o, _)| o) != Some(*opener) {
                            let text = &self.src[tok.span.start..tok.span.end];
                            return Err(malformed(
                                tok.span,
                                format!("`{}` closes no `{}`", text, opener),
                            ));
                        }
                    }
                    end = tok.span.end;
                }
            }
        }
        if let Some((opener, span)) = open.pop() {
            return Err(malformed(span, format!("`{}` is never closed", opener)));
        }
        // `->` with nothing after it, which the body parser would report alike, but which a
        // function without a body would otherwise keep as a declaration no one checks.
        if end == start {
            return Err(Diagnostic::error(
                "E0116",
                self.span(),
                format!("expected a type, found {}", self.found()),
            ));
        }

        Ok(if let Some((bs, be)) = bang_pos {
            let ret = self.src[start..bs].trim().to_string();
            let err = self.src[be..end].trim().to_string();
            (ret, Some(err))
        } else {
            (self.src[start..end].trim().to_string(), None)
        })
    }

    /// A contract clause: to the end of its line, and on past a newline where `spec/grammar.md`,
    /// item 2, says it goes on; it stops before the body's `{` or `=`.
    fn consume_contract_expr(&mut self) -> Result<String, Diagnostic> {
        let start = self.span().start;
        let mut end = start;
        // The brackets open where the scan is, innermost last, with the closer each needs.
        let mut open: Vec<(TokenKind, &str)> = Vec::new();
        let mut last_line = self.span().line;
        let mut last = TokenKind::Requires;
        while *self.peek() != TokenKind::Eof
            && (!open.is_empty() || self.span().line == last_line || continues_statement(&last))
            && !(open.is_empty() && matches!(self.peek(), TokenKind::LBrace | TokenKind::Eq))
        {
            let tok = self.next();
            match tok.kind {
                TokenKind::LParen => open.push((TokenKind::RParen, ")")),
                TokenKind::LBracket => open.push((TokenKind::RBracket, "]")),
                TokenKind::LBrace => open.push((TokenKind::RBrace, "}")),
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => match open.pop() {
                    Some((closer, _)) if closer == tok.kind => {}
                    expected => {
                        let text = &self.src[tok.span.start..tok.span.end];
                        return Err(Diagnostic::error(
                            "E0111",
                            tok.span,
                            match expected {
                                Some((_, closer)) => format!(
                                    "expected `{}` to close a bracket of this contract, found `{}`",
                                    closer, text
                                ),
                                None => format!("`{}` closes no bracket of this contract", text),
                            },
                        ));
                    }
                },
                _ => {}
            }
            last_line = tok.span.line;
            end = tok.span.end;
            last = tok.kind;
        }
        if let Some((_, closer)) = open.pop() {
            return Err(Diagnostic::error(
                "E0111",
                self.span(),
                format!(
                    "expected `{}` to close a bracket of this contract, found {}",
                    closer,
                    self.found()
                ),
            ));
        }
        Ok(self.src[start..end].trim().to_string())
    }
}

/// Whether `tokens` hold attributes alone, `@name` or `@name(...)`, and at least one.
fn attributes_only(tokens: &[Token]) -> bool {
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].kind != TokenKind::At {
            return false;
        }
        i += 2;
        if tokens.get(i).map(|t| &t.kind) == Some(&TokenKind::LParen) {
            match closing(tokens, i) {
                Some(end) => i = end + 1,
                None => return true,
            }
        }
    }
    !tokens.is_empty()
}

/// The index of the token that closes the bracket at `open`, if `tokens` hold it.
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, t) in tokens.iter().enumerate().skip(open) {
        match t.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// A variant in one form, whatever its spacing, line breaks, comments and trailing commas,
/// as a struct's fields are read: its attributes, its name, and its payload's fields
/// `name: type` joined by `, ` in parentheses. Any other form, which the body parser
/// reports, is kept as its tokens, each after a space.
fn canonical_variant(src: &str, tokens: &[Token]) -> String {
    let text = |t: &Token| src.get(t.span.start..t.span.end).unwrap_or("");
    let joined = |ts: &[Token]| ts.iter().map(text).collect::<String>();
    let mut out = String::new();
    let mut i = 0;
    while tokens.get(i).map(|t| &t.kind) == Some(&TokenKind::At) {
        let end = match tokens.get(i + 2).map(|t| &t.kind) {
            Some(TokenKind::LParen) => closing(tokens, i + 2).unwrap_or(tokens.len() - 1),
            _ => (i + 1).min(tokens.len() - 1),
        };
        out.push_str(&joined(&tokens[i..=end]));
        out.push(' ');
        i = end + 1;
    }
    if let Some(name) = tokens.get(i) {
        out.push_str(text(name));
        i += 1;
    }
    if tokens.get(i).map(|t| &t.kind) == Some(&TokenKind::LParen) {
        if let Some(end) = closing(tokens, i) {
            let mut fields: Vec<String> = Vec::new();
            let mut depth = 0i32;
            let mut field: Vec<Token> = Vec::new();
            for t in &tokens[i + 1..end] {
                match t.kind {
                    TokenKind::Comma if depth == 0 => {
                        fields.push(canonical_field(&joined, &field));
                        field.clear();
                        continue;
                    }
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace | TokenKind::Lt => {
                        depth += 1
                    }
                    TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace | TokenKind::Gt => {
                        depth -= 1
                    }
                    TokenKind::Shr => depth -= 2,
                    _ => {}
                }
                field.push(t.clone());
            }
            fields.push(canonical_field(&joined, &field));
            fields.retain(|f| !f.is_empty());
            out.push('(');
            out.push_str(&fields.join(", "));
            out.push(')');
            i = end + 1;
        }
    }
    for t in &tokens[i.min(tokens.len())..] {
        out.push(' ');
        out.push_str(text(t));
    }
    out
}

/// A payload's field, `name: type`, its type's tokens joined; any other form as its tokens.
fn canonical_field(joined: &dyn Fn(&[Token]) -> String, field: &[Token]) -> String {
    match field {
        [name, colon, ty @ ..]
            if matches!(name.kind, TokenKind::Ident(_))
                && colon.kind == TokenKind::Colon
                && !ty.is_empty() =>
        {
            format!("{}: {}", joined(std::slice::from_ref(name)), joined(ty))
        }
        other => joined(other),
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
        assert_eq!(
            kinds("break continue"),
            vec![TokenKind::Break, TokenKind::Continue, TokenKind::Eof]
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
        let (tokens, _) = lex("x 255 y");
        assert_eq!(tokens[2].span.col, 7);
        // Letters right after a float, with no `_`, are a word of their own.
        assert_eq!(
            kinds("1.5ab"),
            vec![
                TokenKind::FloatLit("1.5".into()),
                TokenKind::Ident("ab".into()),
                TokenKind::Eof,
            ]
        );
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
        assert!(lex_errors("val c = 'x'")[0]
            .1
            .contains("no character literals"));
        assert_eq!(
            kinds("<'a>"),
            vec![
                TokenKind::Lt,
                TokenKind::Lifetime("a".into()),
                TokenKind::Gt,
                TokenKind::Eof
            ]
        );
        assert_eq!(lex_errors("'9 x")[0].0, "E0105");
        assert_eq!(kinds("'_a")[0], TokenKind::Lifetime("_a".into()));
        let (tokens, _) = lex("x <'ab> y");
        assert_eq!((tokens[2].span.end, tokens[4].span.col), (6, 9));
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
        let (tokens, errors) = lex(r#"x "\q" z"#);
        assert_eq!(tokens[2].span.col, 8);
        let at = errors[0].primary;
        assert_eq!((at.start, at.end, at.col), (3, 5, 4));
        let at = lex("\"open\nx").1[0].primary;
        assert_eq!((at.start, at.end, at.col), (0, 5, 1));

        // Malformed integers.
        let malformed = [
            ("0x", "`0x` needs at least one hexadecimal digit after it"),
            ("0x_FF", "`0x` needs at least one hexadecimal digit after it"),
            ("0b", "`0b` needs at least one binary digit after it"),
            ("0b102", "`2` is not a binary digit"),
            ("1__0", "a `_` in a number stands between two digits, or starts its suffix (`255_u8`)"),
            ("7_", "a `_` in a number stands between two digits, or starts its suffix (`255_u8`)"),
            ("0b1e", "a literal's suffix starts with `_`: write `0b1_e`"),
            ("1eq5", "a literal's suffix starts with `_`: write `1_eq5`"),
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
        let edit = errors[0].fixes[0].edits[0].span;
        assert_eq!((edit.start, edit.end, edit.col), (9, 9, 10));
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
    fn test_clauses_and_expression_bodies_continue_past_a_newline() {
        let src = "module m\npub fn f(a: [2]i32) -> i32\n  requires a[0] >\n    0\n  ensures (result\n    > 0)\n= a[0] +\n  a[1]\npub fn g() -> i32 = 2\n";
        let module = parse_module(src).unwrap();
        let fns: Vec<&FnDecl> = module
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(&**f),
                _ => None,
            })
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].requires, vec![String::from("a[0] >\n    0")]);
        assert_eq!(fns[0].ensures, vec![String::from("(result\n    > 0)")]);
        assert_eq!(fns[0].body_tokens.len(), 9, "{:?}", fns[0].body_tokens);
        assert_eq!(fns[1].name, "g");
        // An `else` on the next line continues an `if` in an expression body.
        let src = "module m\npub fn h(c: bool) -> i32 = if c { 1 }\n  else { 2 }\npub fn k() -> i32 = 3\n";
        let module = parse_module(src).unwrap();
        assert_eq!(module.items.len(), 2);
        let (_, errors) = parse_functions(src, &module);
        assert!(errors.is_empty(), "{:?}", errors);
        // Brackets hold a body together across lines, and a clause stops where the body starts.
        let src = "module m\npub fn p(a: i32) -> i32 requires a > 0 = add(a,\n  1)\npub fn q(a: i32) -> i32 ensures result > a {\n  a + 1\n}\n";
        let module = parse_module(src).unwrap();
        let fns: Vec<&FnDecl> = module
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(&**f),
                _ => None,
            })
            .collect();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].requires, vec![String::from("a > 0")]);
        assert_eq!(fns[0].body_tokens.len(), 6, "{:?}", fns[0].body_tokens);
        assert_eq!(fns[1].ensures, vec![String::from("result > a")]);
        let (_, errors) = parse_functions(src, &module);
        assert!(errors.is_empty(), "{:?}", errors);
        // Only the open parenthesis keeps this body going past its first line.
        let src = "module m\npub fn r(a: i32) -> i32 = add(a\n  , 1)\npub fn s() -> i32 = 0\n";
        let module = parse_module(src).unwrap();
        assert_eq!(module.items.len(), 2);
        let (_, errors) = parse_functions(src, &module);
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn test_lifetimes_and_async_are_idioms_wherever_they_stand() {
        let message = "Mente writes no lifetimes: a returned view names its source with `from p`, and a parameter that must outlive the call takes the mode `own` or `in`";
        for (src, repaired) in [
            (
                "module m\npub fn first<'a>(x: i32) -> i32 = x\n",
                "module m\npub fn first(x: i32) -> i32 = x\n",
            ),
            (
                "module m\npub struct View<'a> {\n  a: u8\n}\n",
                "module m\npub struct View {\n  a: u8\n}\n",
            ),
            (
                "module m\npub fn f(x: &'a u8) = 1\n",
                "module m\npub fn f(x: &u8) = 1\n",
            ),
        ] {
            let err = parse_module(src).unwrap_err();
            assert_eq!(
                (err.code, err.message.as_str()),
                ("E0120", message),
                "{}",
                src
            );
            assert_eq!(err.fixes[0].applicability, Applicability::Suggestion);
            assert_eq!(
                diagnostics::apply_fix(src, &err.fixes[0]).unwrap(),
                repaired
            );
        }
        for src in [
            "module m\npub fn f() = 'static\n",
            "module m\npub fn f<'a, T>(x: T) = x\n",
        ] {
            let err = parse_module(src).unwrap_err();
            assert_eq!((err.code, err.fixes.len()), ("E0120", 0), "{}", src);
        }

        let src = "module m\npub async fn f() -> i32 = 1\n";
        let err = parse_module(src).unwrap_err();
        assert_eq!(err.code, "E0121");
        let repaired = diagnostics::apply_fix(src, &err.fixes[0]).unwrap();
        assert_eq!(repaired, "module m\npub fn f() -> i32 = 1\n");
        assert!(parse_module(&repaired).is_ok());
    }

    #[test]
    fn test_only_declarations_stand_at_the_top_level() {
        let cases = [
            (
                "module m\nlet x = 1\n",
                "E0111",
                "expected a declaration, such as `fn`, `struct` or `@example`, found `let`",
            ),
            (
                "module m\nimport net\n",
                "E0109",
                "an `import` comes in M6; M2 does not have it",
            ),
            (
                "module m\nextern \"C\" {\n}\n",
                "E0109",
                "an `extern` block comes in M4; M2 does not have it",
            ),
            (
                "module m\nactor A {\n}\n",
                "E0109",
                "an `actor` comes in M7; M2 does not have it",
            ),
        ];
        for (src, code, message) in cases {
            let err = parse_module(src).unwrap_err();
            assert_eq!((err.code, err.message.as_str()), (code, message), "{}", src);
        }
    }

    #[test]
    fn test_tokens_and_trees_dump_in_their_canonical_forms() {
        assert_eq!(
            dump_tokens("pub fn f() = a\\b"),
            "mente-dump 2 tokens\n1:1 Pub pub\n1:5 Fn fn\n1:8 Ident f\n1:9 LParen (\n1:10 RParen )\n1:12 Eq =\n1:14 Ident a\n1:16 Ident b\n"
        );
        assert_eq!(
            dump_tokens("/// a\\b\nmodule m"),
            "mente-dump 2 tokens\n1:1 DocComment /// a\\\\b\n2:1 Module module\n2:8 Ident m\n"
        );
        assert_eq!(
            dump_tree("module m\npub fn f(mut x: u8, y: bool) -> u8\n  requires y\n  ensures result > 0\n  decreases x\n{\n  x\n}\npub fn g() = 1\npub fn h()\n"),
            "mente-dump 2 tree\n(fn f ((mut x u8) (y bool)) u8 ((requires y) (ensures (> result 0)) (decreases x)) {x})\n(fn g () () () (= 1))\n"
        );
        assert_eq!(
            dump_tree("module m\nlet x = 1\n"),
            "mente-dump 2 tree\nerror 2:1 E0111\n"
        );
        // A function nested too deep is reported, not rendered; one with another error in its
        // body is rendered, its error a node of its own.
        let deep = format!(
            "module m\npub fn d(x: u8) -> u8 = x{}\n",
            " + 0".repeat(300)
        );
        assert_eq!(dump_tree(&deep), "mente-dump 2 tree\nerror 2:25 E0127\n");
        assert_eq!(
            dump_tree("module m\npub fn d(x: u8) -> u8 = )\n"),
            "mente-dump 2 tree\n(fn d ((x u8)) u8 () (= <error>))\n"
        );
    }

    /// The declaration parser ends each function where [`split_items`] starts the next item:
    /// an expression body, a contract or a return type that would go on past a newline stops
    /// before a line that begins an item, so a whole check and a session read the same items
    /// (the fuzzer's finding 11).
    #[test]
    fn test_the_declaration_parser_ends_items_where_a_module_splits() {
        for src in [
            "module m\nfn t() =\npub fn g() -> i32 = 0\n",
            "module m\npub fn f(x: i32) -> i32\n  requires x >\npub fn g() -> i32 = 0\n",
            "module m\npub fn f() -> u8\nfn g() -> u8 = 0\n",
            // A line that begins no item still continues the code above it.
            "module m\npub fn f(x: i32) -> i32 = x +\n  1\npub fn g() -> i32 = 0\n",
            "module m\npub fn h(x: i32) -> i32 =\nx + 1\nfn k() -> u8 = 2\n",
            // Spaces and a comment between the last token and the item's line.
            "module m\nfn t() =   // to do\npub fn g() -> i32 = 0\n",
        ] {
            let lines: Vec<u32> = split_items(src)[1..].iter().map(|s| s.line).collect();
            let fns: Vec<u32> = parse_module(src)
                .unwrap()
                .items
                .iter()
                .filter_map(|i| match i {
                    ItemDecl::Fn(f) => Some(f.line),
                    _ => None,
                })
                .collect();
            assert_eq!(fns, lines, "{}", src);
        }
        // A return type ends before a declaration that is no token the scan stops at.
        let m = parse_module("module m\npub fn f() -> u8\nstruct S {\n  x: u8\n}\n").unwrap();
        let [ItemDecl::Fn(f), ItemDecl::Struct(s)] = &m.items[..] else {
            panic!("{:?}", m.items)
        };
        assert_eq!((f.return_ty.as_deref(), s.name.as_str()), (Some("u8"), "S"));
        // An expression body ends at the end of its line when nothing continues it.
        let d = parse_module("module m\npub fn g() -> u8 = 1\n_x\n").unwrap_err();
        assert_eq!((d.code, d.primary.line), ("E0111", 3));
    }

    /// The declaration parser reads each item within the text [`split_items`] gives it, as a
    /// `--watch` session does: whatever a malformed item holds, its error lies within it or at
    /// the next item's first token, never further. Before, a scan that misread one item's end
    /// ran into the next, or to the end of the file (the fuzzer's findings 10, 11, 13 and 18).
    #[test]
    fn test_no_scan_of_one_item_runs_into_the_next() {
        let next = "pub fn g() -> u8 = 1\n";
        for broken in [
            "fn r() -> ]",
            "fn r() -> [4",
            "fn r() -> u8>",
            "fn id() ->",
            "fn t() =",
            "fn e(x: u8) -> u8\n  requires (x",
            "fn e()->2\nrequires(}",
            "fn n(){{{]}}",
            "fn n() {",
        ] {
            let src = format!("module m\n{}\n{}", broken, next);
            // Where the next item starts, as the split finds it: a bracket left open joins `g`
            // to the broken item, as it would a line that continues an expression.
            let g = split_items(&src).get(2).map_or(src.len(), |s| s.offset);
            // The declaration parser's error, or else the body parser's.
            let errors = match parse_module(&src) {
                Err(d) => vec![d],
                Ok(m) => parse_bodies(&src, &m)
                    .into_iter()
                    .flat_map(|p| p.errors)
                    .collect(),
            };
            assert!(!errors.is_empty(), "{}", broken);
            for d in errors {
                assert!(d.primary.start <= g, "{}: {:?}", broken, d);
            }
        }
    }

    /// The brackets a module's text opens count across lines, outside strings and comments, so
    /// that a line in the first column inside a body, a list or an index, such as a comment,
    /// starts no item; a bracket in a string or a comment counts for none, and so does one a
    /// division or an escape stands before.
    #[test]
    fn test_a_split_counts_only_the_brackets_code_opens() {
        let g = "pub fn g() -> u8 = 2\n";
        // The lines the items start on: the first, and `g`'s unless `f` holds it. A comment
        // split off wrongly would take `g` into its own item, so the lines tell, not the count.
        for (src, g_starts) in [
            ("pub fn f() -> u8 {\n// note\n  1\n}\n", true),
            ("pub fn f(\n// note\n  x: u8) -> u8 = x\n", true),
            ("pub fn f(xs: [2]u8) -> u8 = xs[\n// note\n  0]\n", true),
            ("pub fn f() -> u8 {\n  todo(\"}\")\n// note\n  1\n}\n", true),
            ("pub fn f() -> u8 {\n  todo(\"\\\"{\")\n}\n", true),
            ("pub fn f(x: u8) -> u8 { x / (1) }\n", true),
            ("pub fn f() -> u8 { // {\n  1\n}\n", true),
            // A backslash outside a string escapes nothing: its `(` stays open.
            ("pub fn f() -> u8 = \\(\n// note\n", false),
        ] {
            let lines: Vec<u32> = split_items(&format!("{}{}", src, g))
                .iter()
                .map(|s| s.line)
                .collect();
            let g_line = src.lines().count() as u32 + 1;
            let expected: &[u32] = if g_starts { &[1, g_line] } else { &[1] };
            assert_eq!(lines, expected, "{}", src);
        }
    }

    #[test]
    fn test_a_module_splits_into_its_top_level_items() {
        let src = "module m\n\n/// Doc.\n@total\npub fn f(x: u8) -> u8\n  requires x > 0\n{\n  x\n}\n// note\n\npub fn g() = 1\n_x\nfn h() -> u8 {\nval y = 1\ny\n}\n";
        let starts = split_items(src);
        let texts: Vec<&str> = starts
            .iter()
            .enumerate()
            .map(|(i, s)| &src[s.offset..starts.get(i + 1).map_or(src.len(), |n| n.offset)])
            .collect();
        assert_eq!(
            texts,
            [
                "module m\n\n",
                "/// Doc.\n@total\npub fn f(x: u8) -> u8\n  requires x > 0\n{\n  x\n}\n",
                "// note\n\npub fn g() = 1\n_x\n",
                "fn h() -> u8 {\nval y = 1\ny\n}\n",
            ]
        );
        assert_eq!(
            starts[1],
            ItemStart {
                offset: 10,
                line: 3
            }
        );
        assert_eq!(starts[2].line, 10);
        assert_eq!(starts[3].line, 14);
        // A word that begins no declaration continues the item, and so does a line inside
        // brackets, however it is indented; a bracket in a comment or a string counts for none.
        assert_eq!(split_items("module x\nfn\nm\n").len(), 2);
        assert_eq!(split_items("pub fn f() -> u8 =\nx\n").len(), 1);
        assert_eq!(
            split_items("pub fn f() -> u8 {\n  todo(\"{\")\n  // {\n}\npub fn g() = 1\n").len(),
            2
        );
        // An expression body on its own line continues its function.
        assert_eq!(split_items("pub fn f() -> u8\n= 1\n").len(), 1);
        assert_eq!(split_items("").len(), 1);
        assert_eq!(split_items("  indented\n}\n").len(), 1);
    }

    #[test]
    fn test_parse_functions_reads_each_function_with_a_body() {
        let src = "module m\npub fn decl(x: i32) -> i32\npub fn expr(x: i32) -> i32 = x + 1\npub fn block(x: i32) -> i32 {\n  val y = x 1\n  y\n}\npub fn empty() {}\n";
        let module = parse_module(src).unwrap();
        let (functions, errors) = parse_functions(src, &module);
        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["expr", "block", "empty"]);
        assert_eq!(
            errors.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec!["E0112"]
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

    /// A doc comment that ends an item, after its signature on the same line, documents
    /// nothing, and the items after it are parsed as they are when each is parsed alone.
    #[test]
    fn test_a_doc_comment_ending_an_item_leaves_the_items_after_it() {
        let m =
            parse_module("module m\npub fn take(x: u8) /// doc\n\npub fn f() -> u8 = 1\n").unwrap();
        let names: Vec<&str> = m
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["take", "f"]);
        assert!(m.items.iter().all(|i| match i {
            ItemDecl::Fn(f) => f.docs.is_empty(),
            _ => true,
        }));
        let err = parse_module("module m\nfn e()///\nfn").unwrap_err();
        assert_eq!(err.code, "E0101");
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
            [ItemDecl::Fn(f)] => (**f).clone(),
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
        let variants = |code: &str| {
            let m = parse_module(code).unwrap();
            let ItemDecl::Enum(e) = &m.items[0] else {
                panic!("expected an enum")
            };
            e.variants.clone()
        };
        // A variant is read in one form, whatever its spacing, line breaks, comments and
        // trailing commas, as a struct's fields are; an attribute above it is its own.
        for code in [
            "module m\npub enum E {\n  C(w: i128, f: bool)\n}\n",
            "module m\npub enum E {\n  C(w:i128,f:bool)\n}\n",
            "module m\npub enum E {\n  C(\n    w: i128, // wide\n    f: bool,\n  )\n}\n",
        ] {
            assert_eq!(variants(code), ["C(w: i128, f: bool)"], "{}", code);
        }
        assert_eq!(
            variants("module m\npub enum E {\n  @deprecated\n  A\n  @since(2) B(n: [2]u8)\n}\n"),
            ["@deprecated A", "@since(2) B(n: [2]u8)"]
        );
        // Another language's payload in braces stays in its variant, and the enum ends at its
        // own `}`, so the body parser reports it (`E0130`) and the items after it are read.
        let code = "module m\npub enum Shape {\n  Rect { w: u8 }\n  Dot\n}\npub fn f() -> u8 = 0\n";
        let m = parse_module(code).unwrap();
        assert_eq!(m.items.len(), 2);
        assert_eq!(variants(code), ["Rect { w : u8 }", "Dot"]);
        let parsed = parse_enums(code, &m);
        assert_eq!(
            parsed[0].errors.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["E0130"]
        );
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

    /// A return type's brackets and type-argument lists close in order, or the header is
    /// malformed (`E0116`): a closer that closes nothing took the scan past the body, which
    /// left the function a declaration no one checked, and an opener never closed took it past
    /// the items after it.
    /// A contract's brackets close in order, or the header is malformed (`E0111`); inside a
    /// bracket a `{` opens a block, and outside every bracket it begins the body (the fuzzer's
    /// finding 18).
    #[test]
    fn test_a_contracts_brackets_must_close_in_order() {
        for (contract, message) in [
            (
                "requires (}\n= 1",
                "expected `)` to close a bracket of this contract, found `}`",
            ),
            (
                "requires x > 0)\n= 1",
                "`)` closes no bracket of this contract",
            ),
            (
                "requires (x > 0",
                "expected `)` to close a bracket of this contract, found the end of the file",
            ),
        ] {
            let src = format!("module m\nfn e(x: u8) -> u8\n  {}\n", contract);
            let d = parse_module(&src).unwrap_err();
            assert_eq!((d.code, d.message.as_str()), ("E0111", message), "{}", src);
        }
        let f = only_fn(
            "module m\nfn e(x: u8) -> u8\n  requires (if x > 0 { true } else { false })\n{\n  x\n}\n",
        );
        assert_eq!(
            f.requires,
            ["(if x > 0 { true } else { false })".to_string()]
        );
    }

    #[test]
    fn test_a_return_types_brackets_must_close_in_order() {
        for (header, at, what) in [
            ("pub fn f() -> ] = 0", 14, "`]` closes no `[`"),
            ("pub fn f() -> 20]i32 = 0", 16, "`]` closes no `[`"),
            ("pub fn f() -> u8> = 0", 16, "`>` closes no `<`"),
            ("pub fn f() -> List<u8>> = 0", 21, "`>>` closes no `<`"),
            ("pub fn f() -> [4>u8 = 0", 16, "`>` closes no `<`"),
            ("pub fn f() -> [4 = 0", 14, "`[` is never closed"),
            (
                "pub fn f() -> List<u8\npub fn g() -> u8 = 0",
                18,
                "`<` is never closed",
            ),
            ("pub fn f() -> ]", 14, "`]` closes no `[`"),
        ] {
            let d = parse_module(&format!("module m\n{}\n", header)).unwrap_err();
            assert_eq!(d.code, "E0116", "{}", header);
            assert_eq!(d.primary.start, 9 + at, "{}", header);
            assert_eq!(
                d.message,
                format!("the return type of `f` is malformed: {}", what),
                "{}",
                header
            );
        }
        // `->` with no type after it, before another item (the fuzzer's finding 14).
        let d = parse_module("module m\nfn id() ->\nfn f() -> u8 = 1\n").unwrap_err();
        assert_eq!(
            (d.code, d.message.as_str(), d.primary.line),
            ("E0116", "expected a type, found `fn`", 3)
        );
        for (header, ty) in [
            ("pub fn f() -> [4]u8[0..=10]", "[4]u8[0..=10]"),
            ("pub fn f() -> Map<str, List<u8>>", "Map<str, List<u8>>"),
            ("pub fn f() -> Ring<CAP = 64>", "Ring<CAP = 64>"),
        ] {
            let f = only_fn(&format!("module m\n{}\n", header));
            assert_eq!(f.return_ty.as_deref(), Some(ty));
        }
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
    fn test_a_parameter_list_open_at_the_end_of_the_file_is_rejected() {
        let err = parse_module("module m\npub fn f( -> i32 = 1\n").unwrap_err();
        assert_eq!(err.code, "E0111");
        assert_eq!(
            err.message,
            "expected `)` to close this list, found the end of the file"
        );
        assert_eq!((err.related[0].span.line, err.related[0].span.col), (2, 9));
        assert_eq!(err.related[0].message, "the list opens here");
        assert!(parse_module("module m\npub fn f() -> i32 = 1\n").is_ok());
        assert!(parse_module("module m\npub fn f(x: [2]u8) -> u8 = (x[0])\n").is_ok());
        // A function without a parameter list is no function, on one line or two.
        for src in [
            "module x\nfn\nm\n",
            "module x\nfn m\n",
            "module x\nfn m -> u8 = 1\n",
        ] {
            let err = parse_module(src).unwrap_err();
            assert_eq!(err.code, "E0111", "{}", src);
            assert!(
                err.message
                    .starts_with("expected `(` to open the parameter list of `m`"),
                "{}",
                err.message
            );
        }
    }

    /// The declaration parser's cursor stays at `Eof` however often it is moved past it.
    #[test]
    fn test_the_cursor_stays_at_the_end() {
        let src = "module m";
        let (tokens, _) = lex(src);
        let ends = vec![tokens.len() - 1];
        let mut p = Parser {
            src,
            tokens,
            pos: 0,
            last_end: 0,
            ends,
            item: 0,
        };
        let kinds: Vec<TokenKind> = (0..5).map(|_| p.next().kind).collect();
        assert_eq!(
            kinds,
            [
                TokenKind::Module,
                TokenKind::Ident("m".into()),
                TokenKind::Eof,
                TokenKind::Eof,
                TokenKind::Eof
            ]
        );
        assert_eq!((p.peek(), p.span().start), (&TokenKind::Eof, src.len()));
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
            // A function's body the file ends inside (the fuzzer's finding 13).
            (
                "module m\npub fn open() -> u8 {\n  1\n",
                "`fn open` has no closing `}`",
            ),
            ("module m\nfn n(){{{]}}\n", "`fn n` has no closing `}`"),
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

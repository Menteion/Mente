//! A lexer for the part of M1's surface syntax the spike reads (`spec/grammar.md`, item 1),
//! with the newline rules of item 2 applied after lexing.

use crate::Diag;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    Int(i128),
    // Keywords.
    Module,
    Pub,
    Fn,
    Mut,
    Val,
    Var,
    If,
    Else,
    While,
    Break,
    Continue,
    Guard,
    Return,
    Requires,
    Ensures,
    Decreases,
    Invariant,
    And,
    Or,
    Not,
    True,
    False,
    // Punctuation and operators.
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    DotDot,
    DotDotEq,
    Arrow,
    At,
    Semi,
    Eq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    /// A compound assignment: the operator it applies (`+=` is `AssignOp(Plus)`).
    AssignOp(Box<Tok>),
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    /// Byte offsets into the module source.
    pub start: usize,
    pub end: usize,
}

fn keyword(word: &str) -> Option<Tok> {
    Some(match word {
        "module" => Tok::Module,
        "pub" => Tok::Pub,
        "fn" => Tok::Fn,
        "mut" => Tok::Mut,
        "val" => Tok::Val,
        "var" => Tok::Var,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "guard" => Tok::Guard,
        "return" => Tok::Return,
        "requires" => Tok::Requires,
        "ensures" => Tok::Ensures,
        "decreases" => Tok::Decreases,
        "invariant" => Tok::Invariant,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "true" => Tok::True,
        "false" => Tok::False,
        _ => return None,
    })
}

const PUNCT: [(&str, Tok); 22] = [
    ("..=", Tok::DotDotEq),
    ("..", Tok::DotDot),
    ("->", Tok::Arrow),
    ("==", Tok::EqEq),
    ("!=", Tok::NotEq),
    ("<=", Tok::Le),
    (">=", Tok::Ge),
    ("(", Tok::LParen),
    (")", Tok::RParen),
    ("[", Tok::LBracket),
    ("]", Tok::RBracket),
    ("{", Tok::LBrace),
    ("}", Tok::RBrace),
    (",", Tok::Comma),
    (":", Tok::Colon),
    (".", Tok::Dot),
    ("@", Tok::At),
    (";", Tok::Semi),
    ("=", Tok::Eq),
    ("<", Tok::Lt),
    (">", Tok::Gt),
    ("~", Tok::Tilde),
];

/// Binary operators that may be followed by `=` to form a compound assignment.
const OPERATORS: [(&str, Tok); 10] = [
    ("<<", Tok::Shl),
    (">>", Tok::Shr),
    ("+", Tok::Plus),
    ("-", Tok::Minus),
    ("*", Tok::Star),
    ("/", Tok::Slash),
    ("%", Tok::Percent),
    ("&", Tok::Amp),
    ("|", Tok::Pipe),
    ("^", Tok::Caret),
];

/// Lexes `src`, whose first byte is at offset `base` of the module source.
pub fn lex(src: &str, base: usize) -> Result<Vec<Token>, Diag> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        if c == b'\n' {
            out.push(Token {
                tok: Tok::Newline,
                start: base + i,
                end: base + i + 1,
            });
            i += 1;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            let mut value: i128 = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                if bytes[i] != b'_' {
                    value = value
                        .checked_mul(10)
                        .and_then(|v| v.checked_add(i128::from(bytes[i] - b'0')))
                        .ok_or_else(|| {
                            Diag::new(base + start, base + i, "integer literal too large")
                        })?;
                }
                i += 1;
            }
            if i < bytes.len() && (bytes[i].is_ascii_alphabetic()) {
                return Err(Diag::new(
                    base + start,
                    base + i + 1,
                    "the spike reads decimal literals without a suffix only",
                ));
            }
            out.push(Token {
                tok: Tok::Int(value),
                start: base + start,
                end: base + i,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let tok = keyword(word).unwrap_or_else(|| Tok::Ident(word.to_string()));
            out.push(Token {
                tok,
                start: base + start,
                end: base + i,
            });
            continue;
        }
        let arrow = src[i..].starts_with("->");
        if let Some((text, tok)) = OPERATORS
            .iter()
            .find(|(t, _)| !arrow && src[i..].starts_with(t))
        {
            i += text.len();
            let tok = if i < bytes.len() && bytes[i] == b'=' && !src[i..].starts_with("==") {
                i += 1;
                Tok::AssignOp(Box::new(tok.clone()))
            } else {
                tok.clone()
            };
            out.push(Token {
                tok,
                start: base + start,
                end: base + i,
            });
            continue;
        }
        if let Some((text, tok)) = PUNCT.iter().find(|(t, _)| src[i..].starts_with(t)) {
            i += text.len();
            out.push(Token {
                tok: tok.clone(),
                start: base + start,
                end: base + i,
            });
            continue;
        }
        return Err(Diag::new(
            base + start,
            base + start + 1,
            "a character the lexer does not know",
        ));
    }
    out.push(Token {
        tok: Tok::Eof,
        start: base + bytes.len(),
        end: base + bytes.len(),
    });
    Ok(fold_newlines(out))
}

/// A token after which a newline does not end a statement (`spec/grammar.md`, item 2).
fn continues(tok: &Tok) -> bool {
    matches!(
        tok,
        Tok::Plus
            | Tok::Minus
            | Tok::Star
            | Tok::Slash
            | Tok::Percent
            | Tok::Amp
            | Tok::Pipe
            | Tok::Caret
            | Tok::Shl
            | Tok::Shr
            | Tok::EqEq
            | Tok::NotEq
            | Tok::Lt
            | Tok::Le
            | Tok::Gt
            | Tok::Ge
            | Tok::And
            | Tok::Or
            | Tok::Not
            | Tok::Eq
            | Tok::AssignOp(_)
            | Tok::Comma
            | Tok::Dot
            | Tok::Arrow
            | Tok::LParen
            | Tok::LBracket
            | Tok::LBrace
    )
}

/// A token before which a newline does not end a statement.
fn continued_by(tok: &Tok) -> bool {
    matches!(
        tok,
        Tok::LBrace | Tok::Else | Tok::Invariant | Tok::Decreases | Tok::Requires | Tok::Ensures
    )
}

/// Applies the newline rules: newlines inside parentheses and brackets, after a token that cannot
/// end a statement, before a line that continues one, and repeated newlines are dropped.
fn fold_newlines(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut depth: i32 = 0;
    for (k, t) in tokens.iter().enumerate() {
        match t.tok {
            Tok::LParen | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBracket => depth -= 1,
            _ => {}
        }
        if t.tok == Tok::Newline {
            let prev_continues = out
                .last()
                .is_none_or(|p| continues(&p.tok) || p.tok == Tok::Newline);
            let next = tokens[k + 1..].iter().find(|n| n.tok != Tok::Newline);
            let next_continues = next.is_some_and(|n| continued_by(&n.tok) || n.tok == Tok::Eof);
            if depth > 0 || prev_continues || next_continues {
                continue;
            }
        }
        out.push(t.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src, 0).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn test_compound_assignment_and_comparison_are_distinct() {
        assert_eq!(
            kinds("x += 1"),
            vec![
                Tok::Ident("x".into()),
                Tok::AssignOp(Box::new(Tok::Plus)),
                Tok::Int(1),
                Tok::Eof
            ]
        );
        assert_eq!(
            kinds("a <= b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Le,
                Tok::Ident("b".into()),
                Tok::Eof
            ]
        );
        assert_eq!(
            kinds("a <<= 2"),
            vec![
                Tok::Ident("a".into()),
                Tok::AssignOp(Box::new(Tok::Shl)),
                Tok::Int(2),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn test_newlines_follow_the_grammar() {
        // Inside brackets, after an operator and before `else` or `{`, a newline is whitespace.
        let toks = kinds("val x = a +\n  b\nf(1,\n 2)\nif c {\n}\nelse {\n}\n");
        let newlines = toks.iter().filter(|t| **t == Tok::Newline).count();
        assert_eq!(newlines, 2, "{toks:?}");
    }

    #[test]
    fn test_spans_are_offsets_into_the_module() {
        let toks = lex("val x", 100).unwrap();
        assert_eq!((toks[1].start, toks[1].end), (104, 105));
    }

    #[test]
    fn test_unknown_characters_and_suffixes_are_errors() {
        assert!(lex("x $ y", 0).is_err());
        assert!(lex("255_u8", 0).is_err());
    }
}

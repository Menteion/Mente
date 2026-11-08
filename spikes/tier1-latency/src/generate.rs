//! The generated module of the Tier-1 protocol: functions of 20 obligations each (loop bounds,
//! array indexing and a chain of `requires`), and random single-token edits inside their bodies.

use crate::lexer::{lex, Tok};

/// A small deterministic generator (xorshift64).
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// Each function walks part of an array twice, then calls the previous function on a narrower
/// range, whose `requires` it must establish; the first calls the last, so every function has
/// the same shape and the same 20 obligations.
pub fn module(functions: usize, seed: u64) -> String {
    let mut r = Rng(seed | 1);
    let mut out = String::from("module generated.tier1\n\n");
    for k in 0..functions {
        let callee = (k + functions - 1) % functions;
        let c1 = r.below(2001) as i64 - 1000;
        let c2 = r.below(2001) as i64 - 1000;
        out.push_str(&format!(
            "/// Link {k} of the chain.\n\
             fn f{k}(a: [64]i32, lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n    \
                 requires lo < hi\n    \
                 ensures lo <= result and result <= hi\n\
             {{\n    \
                 var i: usize = lo\n    \
                 var s: i64 = 0\n    \
                 while i < hi invariant lo <= i {{\n        \
                     s += a[i]\n        \
                     i += 1\n    \
                 }}\n    \
                 var j: usize = hi\n    \
                 while j > lo invariant j <= hi {{\n        \
                     j -= 1\n        \
                     guard a[j] != {c1} else {{ break }}\n    \
                 }}\n    \
                 guard lo + 1 < hi else {{ return lo }}\n    \
                 val r: usize = f{callee}(a, lo + 1, hi)\n    \
                 if s > {c2} {{ return r }}\n    \
                 return hi\n\
             }}\n\n"
        ));
    }
    out
}

/// One single-token edit: replace `src[start..end]` with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

const NAMES: [&str; 6] = ["lo", "hi", "i", "j", "s", "r"];
const COMPARISONS: [&str; 6] = ["<", "<=", ">", ">=", "==", "!="];

/// A random single-token edit inside the body of the function spanning `body_start..body_end`:
/// an integer literal, a comparison, `+` or `-` (also in `+=` and `-=`), or a variable name
/// replaced by another of its kind.
pub fn random_edit(src: &str, body_start: usize, body_end: usize, r: &mut Rng) -> Option<Edit> {
    let toks = lex(&src[body_start..body_end], body_start).ok()?;
    let candidates: Vec<_> = toks
        .iter()
        .filter(|t| match &t.tok {
            Tok::Int(_)
            | Tok::Lt
            | Tok::Le
            | Tok::Gt
            | Tok::Ge
            | Tok::EqEq
            | Tok::NotEq
            | Tok::Plus
            | Tok::Minus => true,
            Tok::AssignOp(op) => matches!(**op, Tok::Plus | Tok::Minus),
            Tok::Ident(n) => NAMES.contains(&n.as_str()),
            _ => false,
        })
        .collect();
    let t = candidates.get(r.below(candidates.len()))?;
    let old = &src[t.start..t.end];
    let text = match &t.tok {
        Tok::Int(v) => {
            let choices = [0, 1, 2, 63, 64, 65, v - 1, v + 1];
            choices
                .iter()
                .filter(|c| **c >= 0)
                .map(|c| c.to_string())
                .filter(|c| c != old)
                .nth(r.below(6))?
        }
        Tok::Plus => "-".to_string(),
        Tok::Minus => "+".to_string(),
        Tok::AssignOp(op) if **op == Tok::Plus => "-=".to_string(),
        Tok::AssignOp(_) => "+=".to_string(),
        Tok::Ident(_) => NAMES
            .iter()
            .filter(|n| **n != old)
            .nth(r.below(5))?
            .to_string(),
        _ => COMPARISONS
            .iter()
            .filter(|c| **c != old)
            .nth(r.below(5))?
            .to_string(),
    };
    Some(Edit {
        start: t.start,
        end: t.end,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edits_change_exactly_one_token() {
        let src = module(3, 7);
        let mut r = Rng(42);
        for _ in 0..200 {
            let e = random_edit(&src, 0, src.len(), &mut r).unwrap();
            assert_ne!(&src[e.start..e.end], e.text);
            let mut edited = src.clone();
            edited.replace_range(e.start..e.end, &e.text);
            let before = lex(&src, 0).unwrap().len();
            let after = lex(&edited, 0).unwrap().len();
            assert_eq!(before, after, "{e:?}");
        }
    }
}

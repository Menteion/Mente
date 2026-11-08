//! A recursive-descent parser for the part of M1 the spike reads, following `spec/grammar.md`.

use crate::ast::*;
use crate::lexer::{lex, Tok, Token};
use crate::Diag;

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

/// FNV-1a over the source text of a run of tokens: a digest that ignores whitespace and comments.
fn digest(src: &str, base: usize, toks: &[Token]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for t in toks {
        for b in src.as_bytes()[t.start - base..t.end - base]
            .iter()
            .chain([0u8].iter())
        {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Parses every function of a module.
pub fn parse_module(src: &str) -> Result<Vec<Function>, Diag> {
    let mut p = Parser {
        toks: lex(src, 0)?,
        pos: 0,
    };
    let mut out = Vec::new();
    p.skip_newlines();
    if p.eat(&Tok::Module) {
        p.ident()?;
        while p.eat(&Tok::Dot) {
            p.ident()?;
        }
    }
    loop {
        p.skip_newlines();
        if p.at(&Tok::Eof) {
            return Ok(out);
        }
        out.push(p.function(src, 0)?);
    }
}

/// Parses the one function that `src[start..end]` holds; spans stay offsets into `src`.
pub fn parse_function_at(src: &str, start: usize, end: usize) -> Result<Function, Diag> {
    let text = &src[start..end];
    let mut p = Parser {
        toks: lex(text, start)?,
        pos: 0,
    };
    p.skip_newlines();
    let f = p.function(text, start)?;
    p.skip_newlines();
    if !p.at(&Tok::Eof) {
        return Err(p.error("text after the end of the function"));
    }
    Ok(f)
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }
    fn token(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }
    fn bump(&mut self) -> Token {
        let t = self.token().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn error(&self, msg: &str) -> Diag {
        let t = self.token();
        Diag::new(t.start, t.end, msg)
    }
    fn expect(&mut self, t: &Tok, what: &str) -> Result<Token, Diag> {
        if self.at(t) {
            Ok(self.bump())
        } else {
            Err(self.error(&format!("expected {what}")))
        }
    }
    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) {
            self.bump();
        }
    }
    fn ident(&mut self) -> Result<String, Diag> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            _ => Err(self.error("expected a name")),
        }
    }
    fn int(&mut self) -> Result<i128, Diag> {
        let neg = self.eat(&Tok::Minus);
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(if neg { -v } else { v })
            }
            _ => Err(self.error("expected an integer literal")),
        }
    }

    fn function(&mut self, src: &str, base: usize) -> Result<Function, Diag> {
        let start = self.token().start;
        let head_from = self.pos;
        self.eat(&Tok::Pub);
        self.expect(&Tok::Fn, "fn")?;
        let name = self.ident()?;
        self.expect(&Tok::LParen, "(")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            if self.at(&Tok::Mut) {
                return Err(self.error("the spike reads read-only parameters only"));
            }
            let pname = self.ident()?;
            self.expect(&Tok::Colon, ":")?;
            params.push(Param {
                name: pname,
                ty: self.ty()?,
            });
            if !self.at(&Tok::RParen) {
                self.expect(&Tok::Comma, ", or )")?;
            }
        }
        self.bump();
        let ret = if self.eat(&Tok::Arrow) {
            self.ty()?
        } else {
            Ty::Unit
        };
        let (mut requires, mut ensures) = (Vec::new(), Vec::new());
        loop {
            if self.eat(&Tok::Requires) {
                requires.push(self.expr()?);
            } else if self.eat(&Tok::Ensures) {
                ensures.push(self.expr()?);
            } else if self.eat(&Tok::Decreases) {
                self.expr()?;
            } else {
                break;
            }
        }
        let head_hash = digest(src, base, &self.toks[head_from..self.pos]);
        let body_from = self.pos;
        let body_start = self.token().start;
        let body = self.block()?;
        let end = self.toks[self.pos - 1].end;
        let body_hash = digest(src, base, &self.toks[body_from..self.pos]);
        Ok(Function {
            sig: Signature {
                name,
                params,
                ret,
                requires,
                ensures,
            },
            body,
            span: Span { start, end },
            header_hash: head_hash,
            body_hash,
            body_start,
        })
    }

    fn ty(&mut self) -> Result<Ty, Diag> {
        if self.eat(&Tok::LBracket) {
            let len = self.int()?;
            self.expect(&Tok::RBracket, "]")?;
            return Ok(Ty::Array {
                len,
                elem: Box::new(self.ty()?),
            });
        }
        let name = self.ident()?;
        if name == "bool" {
            return Ok(Ty::Bool);
        }
        let base = int_base(&name)
            .ok_or_else(|| self.error(&format!("the spike does not know the type {name}")))?;
        let refined = if self.eat(&Tok::LBracket) {
            let lo = self.int()?;
            let inclusive = self.eat(&Tok::DotDotEq);
            if !inclusive {
                self.expect(&Tok::DotDot, ".. or ..=")?;
            }
            let hi = self.int()? - if inclusive { 0 } else { 1 };
            self.expect(&Tok::RBracket, "]")?;
            if lo < base.lo || hi > base.hi || lo > hi {
                return Err(self.error("a refinement outside its type, or empty"));
            }
            Some(Range { lo, hi })
        } else {
            None
        };
        Ok(Ty::Int {
            name,
            base,
            refined,
        })
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Diag> {
        self.expect(&Tok::LBrace, "{")?;
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            if self.eat(&Tok::RBrace) {
                return Ok(out);
            }
            out.push(self.stmt()?);
            if !self.at(&Tok::Newline) && !self.at(&Tok::RBrace) {
                return Err(self.error("expected the end of the statement"));
            }
        }
    }

    fn stmt(&mut self) -> Result<Stmt, Diag> {
        let start = self.token().start;
        let kind = match self.peek().clone() {
            Tok::Val | Tok::Var => {
                let mutable = self.bump().tok == Tok::Var;
                let name = self.ident()?;
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect(&Tok::Eq, "=")?;
                StmtKind::Let {
                    mutable,
                    name,
                    ty,
                    init: self.expr()?,
                }
            }
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let (mut invariants, mut decreases) = (Vec::new(), None);
                loop {
                    if self.eat(&Tok::Invariant) {
                        invariants.push(self.expr()?);
                    } else if self.eat(&Tok::Decreases) {
                        decreases = Some(self.expr()?);
                    } else {
                        break;
                    }
                }
                StmtKind::While {
                    cond,
                    invariants,
                    decreases,
                    body: self.block()?,
                }
            }
            Tok::If => return self.if_stmt(),
            Tok::Guard => {
                self.bump();
                let cond = self.expr()?;
                self.expect(&Tok::Else, "else")?;
                StmtKind::Guard {
                    cond,
                    body: self.block()?,
                }
            }
            Tok::Return => {
                self.bump();
                if self.at(&Tok::Newline) || self.at(&Tok::RBrace) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()?))
                }
            }
            Tok::Break => {
                self.bump();
                StmtKind::Break
            }
            Tok::Continue => {
                self.bump();
                StmtKind::Continue
            }
            _ => {
                let e = self.expr()?;
                let op = match self.peek().clone() {
                    Tok::Eq => Some(None),
                    Tok::AssignOp(op) => Some(Some(
                        binop(&op).ok_or_else(|| self.error("not an assignment operator"))?,
                    )),
                    _ => None,
                };
                match op {
                    None => StmtKind::Expr(e),
                    Some(op) => {
                        self.bump();
                        let target = place(&e).ok_or_else(|| {
                            Diag::new(
                                e.span.start,
                                e.span.end,
                                "only a name or an element can be assigned",
                            )
                        })?;
                        StmtKind::Assign {
                            target,
                            op,
                            value: self.expr()?,
                        }
                    }
                }
            }
        };
        Ok(Stmt {
            kind,
            span: Span {
                start,
                end: self.toks[self.pos - 1].end,
            },
        })
    }

    fn if_stmt(&mut self) -> Result<Stmt, Diag> {
        let start = self.bump().start;
        let cond = self.expr()?;
        let then = self.block()?;
        let els = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                vec![self.if_stmt()?]
            } else {
                self.block()?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt {
            kind: StmtKind::If { cond, then, els },
            span: Span {
                start,
                end: self.toks[self.pos - 1].end,
            },
        })
    }

    pub fn expr(&mut self) -> Result<Expr, Diag> {
        self.or()
    }

    fn binary(&mut self, op: BinOp, l: Expr, r: Expr) -> Expr {
        let span = Span {
            start: l.span.start,
            end: r.span.end,
        };
        Expr {
            kind: ExprKind::Binary(op, Box::new(l), Box::new(r)),
            span,
        }
    }

    fn or(&mut self) -> Result<Expr, Diag> {
        let mut l = self.and()?;
        while self.eat(&Tok::Or) {
            let r = self.and()?;
            l = self.binary(BinOp::Or, l, r);
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Expr, Diag> {
        let mut l = self.not()?;
        while self.eat(&Tok::And) {
            let r = self.not()?;
            l = self.binary(BinOp::And, l, r);
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Expr, Diag> {
        if self.at(&Tok::Not) {
            let start = self.bump().start;
            let e = self.not()?;
            let span = Span {
                start,
                end: e.span.end,
            };
            return Ok(Expr {
                kind: ExprKind::Unary(UnOp::Not, Box::new(e)),
                span,
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, Diag> {
        let l = self.level(0)?;
        let op = match self.peek() {
            Tok::EqEq => BinOp::Eq,
            Tok::NotEq => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            _ => return Ok(l),
        };
        self.bump();
        let r = self.level(0)?;
        if matches!(
            self.peek(),
            Tok::EqEq | Tok::NotEq | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge
        ) {
            return Err(self.error("comparisons do not chain: write a < b and b < c"));
        }
        Ok(self.binary(op, l, r))
    }

    /// The binary levels below comparisons, loosest first: `|`, `^`, `&`, shifts, `+ -`, `* / %`.
    fn level(&mut self, n: usize) -> Result<Expr, Diag> {
        const LEVELS: [&[(Tok, BinOp)]; 6] = [
            &[(Tok::Pipe, BinOp::BitOr)],
            &[(Tok::Caret, BinOp::BitXor)],
            &[(Tok::Amp, BinOp::BitAnd)],
            &[(Tok::Shl, BinOp::Shl), (Tok::Shr, BinOp::Shr)],
            &[(Tok::Plus, BinOp::Add), (Tok::Minus, BinOp::Sub)],
            &[
                (Tok::Star, BinOp::Mul),
                (Tok::Slash, BinOp::Div),
                (Tok::Percent, BinOp::Rem),
            ],
        ];
        if n == LEVELS.len() {
            return self.unary();
        }
        let mut l = self.level(n + 1)?;
        while let Some((_, op)) = LEVELS[n].iter().find(|(t, _)| self.at(t)) {
            let op = *op;
            self.bump();
            let r = self.level(n + 1)?;
            l = self.binary(op, l, r);
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr, Diag> {
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Tilde => UnOp::BitNot,
            _ => return self.postfix(),
        };
        let start = self.bump().start;
        let e = self.unary()?;
        let span = Span {
            start,
            end: e.span.end,
        };
        if let (UnOp::Neg, ExprKind::Int(v)) = (op, &e.kind) {
            return Ok(Expr {
                kind: ExprKind::Int(-v),
                span,
            });
        }
        Ok(Expr {
            kind: ExprKind::Unary(op, Box::new(e)),
            span,
        })
    }

    fn postfix(&mut self) -> Result<Expr, Diag> {
        let mut e = self.primary()?;
        loop {
            if self.eat(&Tok::LBracket) {
                let i = self.expr()?;
                let end = self.expect(&Tok::RBracket, "]")?.end;
                let span = Span {
                    start: e.span.start,
                    end,
                };
                e = Expr {
                    kind: ExprKind::Index(Box::new(e), Box::new(i)),
                    span,
                };
            } else if self.eat(&Tok::Dot) {
                let t = self.token().clone();
                if self.ident()? != "len" {
                    return Err(Diag::new(
                        t.start,
                        t.end,
                        "in M1 `.len` is the only use of `.`",
                    ));
                }
                let span = Span {
                    start: e.span.start,
                    end: t.end,
                };
                e = Expr {
                    kind: ExprKind::Len(Box::new(e)),
                    span,
                };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Diag> {
        let t = self.bump();
        let kind = match t.tok {
            Tok::Int(v) => ExprKind::Int(v),
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::Ident(name) => {
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while !self.at(&Tok::RParen) {
                        if self.at(&Tok::Mut) {
                            return Err(self.error("the spike reads read-only arguments only"));
                        }
                        args.push(self.expr()?);
                        if !self.at(&Tok::RParen) {
                            self.expect(&Tok::Comma, ", or )")?;
                        }
                    }
                    self.bump();
                    ExprKind::Call(name, args)
                } else {
                    ExprKind::Name(name)
                }
            }
            Tok::LParen => {
                let e = self.expr()?;
                let end = self.expect(&Tok::RParen, ")")?.end;
                return Ok(Expr {
                    kind: e.kind,
                    span: Span {
                        start: t.start,
                        end,
                    },
                });
            }
            Tok::LBracket => {
                let v = self.expr()?;
                self.expect(
                    &Tok::Semi,
                    "; (the spike reads only the array repeat [v; N])",
                )?;
                let n = self.int()?;
                self.expect(&Tok::RBracket, "]")?;
                ExprKind::ArrayRepeat(Box::new(v), n)
            }
            _ => return Err(Diag::new(t.start, t.end, "expected an expression")),
        };
        Ok(Expr {
            kind,
            span: Span {
                start: t.start,
                end: self.toks[self.pos - 1].end,
            },
        })
    }
}

fn binop(t: &Tok) -> Option<BinOp> {
    Some(match t {
        Tok::Plus => BinOp::Add,
        Tok::Minus => BinOp::Sub,
        Tok::Star => BinOp::Mul,
        Tok::Slash => BinOp::Div,
        Tok::Percent => BinOp::Rem,
        Tok::Amp => BinOp::BitAnd,
        Tok::Pipe => BinOp::BitOr,
        Tok::Caret => BinOp::BitXor,
        Tok::Shl => BinOp::Shl,
        Tok::Shr => BinOp::Shr,
        _ => return None,
    })
}

fn place(e: &Expr) -> Option<Place> {
    match &e.kind {
        ExprKind::Name(n) => Some(Place {
            name: n.clone(),
            indices: Vec::new(),
            span: e.span,
        }),
        ExprKind::Index(a, i) => {
            let mut p = place(a)?;
            p.indices.push((**i).clone());
            p.span = e.span;
            Some(p)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "module demo\n\n/// Doc.\nfn f(a: [8]i32, n: usize[0..=8]) -> i64\n    requires n > 0\n    ensures result >= 0\n{\n    var i: usize = 0\n    while i < n invariant i <= n {\n        i += 1\n    }\n    guard n < 8 else { return 0 }\n    if a[0] > 1 { return 1 }\n    else { return 2 }\n}\n";

    #[test]
    fn test_a_function_parses_with_its_contracts_and_clauses() {
        let fs = parse_module(SRC).unwrap();
        assert_eq!(fs.len(), 1);
        let f = &fs[0];
        assert_eq!(f.sig.name, "f");
        assert_eq!(
            f.sig.params[1].ty,
            Ty::Int {
                name: "usize".into(),
                base: int_base("usize").unwrap(),
                refined: Some(Range { lo: 0, hi: 8 })
            }
        );
        assert_eq!((f.sig.requires.len(), f.sig.ensures.len()), (1, 1));
        assert_eq!(f.body.len(), 4);
        assert!(
            matches!(&f.body[1].kind, StmtKind::While { invariants, .. } if invariants.len() == 1)
        );
        assert!(matches!(&f.body[3].kind, StmtKind::If { els, .. } if els.len() == 1));
        assert_eq!(&SRC[f.span.start..f.span.end][..2], "fn");
    }

    #[test]
    fn test_one_function_reparses_to_the_same_tree() {
        let fs = parse_module(SRC).unwrap();
        let again = parse_function_at(SRC, fs[0].span.start, fs[0].span.end).unwrap();
        assert_eq!(again, fs[0]);
    }

    #[test]
    fn test_precedence_follows_the_grammar() {
        let src = "fn g(x: i32) -> bool {\n    return not x & 3 == 0 or x < 1 and x > -5\n}\n";
        let f = &parse_module(src).unwrap()[0];
        let StmtKind::Return(Some(e)) = &f.body[0].kind else {
            panic!()
        };
        // or( not( (x & 3) == 0 ), and(...) )
        let ExprKind::Binary(BinOp::Or, l, _) = &e.kind else {
            panic!("{e:?}")
        };
        let ExprKind::Unary(UnOp::Not, inner) = &l.kind else {
            panic!("{l:?}")
        };
        let ExprKind::Binary(BinOp::Eq, band, _) = &inner.kind else {
            panic!("{inner:?}")
        };
        assert!(matches!(band.kind, ExprKind::Binary(BinOp::BitAnd, _, _)));
    }

    #[test]
    fn test_chained_comparisons_are_rejected() {
        let src = "fn g(x: i32) -> bool {\n    return 0 < x < 5\n}\n";
        assert!(parse_module(src)
            .unwrap_err()
            .message
            .contains("do not chain"));
    }

    #[test]
    fn test_the_body_digest_ignores_whitespace_and_the_header_digest_sees_contracts() {
        let a = &parse_module("fn f(x: i32) -> i32\n    requires x > 0\n{\n    return x\n}\n")
            .unwrap()[0];
        let b = &parse_module(
            "fn f(x: i32) -> i32\n    requires x > 0\n{\n    return   x // same\n}\n",
        )
        .unwrap()[0];
        let c = &parse_module("fn f(x: i32) -> i32\n    requires x > 1\n{\n    return x\n}\n")
            .unwrap()[0];
        assert_eq!(a.body_hash, b.body_hash);
        assert_eq!(a.header_hash, b.header_hash);
        assert_ne!(a.header_hash, c.header_hash);
        assert_eq!(a.body_hash, c.body_hash);
    }
}

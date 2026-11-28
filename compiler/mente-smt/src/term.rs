//! The terms of a query: a table of nodes, each made once (two equal nodes are one), which a
//! query writes as one `define-fun` each, children before their parents.
//!
//! The nodes are SMT-LIB's own operations over integers, booleans and arrays indexed by
//! integers. A bit operation is written through bit-vectors, `(bv2nat (bvand ((_ int2bv 8) x)
//! ((_ int2bv 8) y)))`: the operands' two's-complement bits in a width, and the unsigned value of
//! the result. Operations on constants alone are folded where they are made, so that a path no
//! input can take shows as the constant `false`.

use std::collections::HashMap;
use std::fmt::Write;

use crate::num::Big;

/// A node of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct T(pub u32);

/// The sort of a term: an integer, a boolean, or an array from integers to a sort.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Sort {
    Int,
    Bool,
    Array(Box<Sort>),
}

impl Sort {
    /// The sort as SMT-LIB writes it: `Int`, `(Array Int Bool)`.
    pub fn smt(&self) -> String {
        match self {
            Sort::Int => "Int".to_string(),
            Sort::Bool => "Bool".to_string(),
            Sort::Array(e) => format!("(Array Int {})", e.smt()),
        }
    }
}

/// A bit operation on two's-complement bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bits {
    And,
    Or,
    Xor,
    Not,
}

impl Bits {
    fn smt(self) -> &'static str {
        match self {
            Bits::And => "bvand",
            Bits::Or => "bvor",
            Bits::Xor => "bvxor",
            Bits::Not => "bvnot",
        }
    }
}

/// An operation over terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Add,
    Sub,
    Mul,
    /// SMT-LIB's `div`, whose remainder is never negative.
    Div,
    /// SMT-LIB's `mod`.
    Mod,
    Le,
    Lt,
    Eq,
    Not,
    And,
    Or,
    Ite,
    Select,
    Store,
    /// The unsigned value of a bit operation on its operands' bits in a width.
    Bits(Bits, u32),
}

impl Op {
    fn smt(self) -> String {
        match self {
            Op::Add => "+".into(),
            Op::Sub => "-".into(),
            Op::Mul => "*".into(),
            Op::Div => "div".into(),
            Op::Mod => "mod".into(),
            Op::Le => "<=".into(),
            Op::Lt => "<".into(),
            Op::Eq => "=".into(),
            Op::Not => "not".into(),
            Op::And => "and".into(),
            Op::Or => "or".into(),
            Op::Ite => "ite".into(),
            Op::Select => "select".into(),
            Op::Store => "store".into(),
            Op::Bits(b, _) => b.smt().into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Num(Big),
    Bool(bool),
    /// An input of the query, declared with `declare-const`.
    Input(String, Sort),
    App(Op, Vec<T>),
    /// The array whose every element is the term, of the sort given.
    Const(Sort, T),
}

/// The table of terms of one function's encoding.
#[derive(Clone, Debug, Default)]
pub struct Terms {
    nodes: Vec<Node>,
    sorts: Vec<Sort>,
    index: HashMap<Node, T>,
}

impl Terms {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, t: T) -> &Node {
        &self.nodes[t.0 as usize]
    }

    pub fn sort(&self, t: T) -> &Sort {
        &self.sorts[t.0 as usize]
    }

    fn make(&mut self, node: Node, sort: Sort) -> T {
        if let Some(&t) = self.index.get(&node) {
            return t;
        }
        let t = T(self.nodes.len() as u32);
        self.nodes.push(node.clone());
        self.sorts.push(sort);
        self.index.insert(node, t);
        t
    }

    pub fn num(&mut self, v: Big) -> T {
        self.make(Node::Num(v), Sort::Int)
    }

    pub fn int(&mut self, v: i128) -> T {
        self.num(Big::from_i128(v))
    }

    pub fn bool(&mut self, b: bool) -> T {
        self.make(Node::Bool(b), Sort::Bool)
    }

    pub fn input(&mut self, name: String, sort: Sort) -> T {
        self.make(Node::Input(name, sort.clone()), sort)
    }

    /// The constant an integer term is, if it is one.
    pub fn as_num(&self, t: T) -> Option<&Big> {
        match self.node(t) {
            Node::Num(v) => Some(v),
            Node::Bool(_) | Node::Input(..) | Node::App(..) | Node::Const(..) => None,
        }
    }

    /// The constant a boolean term is, if it is one.
    pub fn as_bool(&self, t: T) -> Option<bool> {
        match self.node(t) {
            Node::Bool(b) => Some(*b),
            Node::Num(_) | Node::Input(..) | Node::App(..) | Node::Const(..) => None,
        }
    }

    fn app(&mut self, op: Op, args: Vec<T>, sort: Sort) -> T {
        self.make(Node::App(op, args), sort)
    }

    /// An arithmetic operation, folded on constants.
    fn arith(&mut self, op: Op, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_num(a), self.as_num(b)) {
            let v = match op {
                Op::Add => Some(x.add(y)),
                Op::Sub => Some(x.sub(y)),
                Op::Mul => Some(x.mul(y)),
                Op::Div => x.div_mod(y).map(|(q, _)| q),
                Op::Mod => x.div_mod(y).map(|(_, r)| r),
                Op::Le
                | Op::Lt
                | Op::Eq
                | Op::Not
                | Op::And
                | Op::Or
                | Op::Ite
                | Op::Select
                | Op::Store
                | Op::Bits(..) => None,
            };
            if let Some(v) = v {
                return self.num(v);
            }
        }
        self.app(op, vec![a, b], Sort::Int)
    }

    pub fn add(&mut self, a: T, b: T) -> T {
        self.arith(Op::Add, a, b)
    }

    pub fn sub(&mut self, a: T, b: T) -> T {
        self.arith(Op::Sub, a, b)
    }

    pub fn mul(&mut self, a: T, b: T) -> T {
        self.arith(Op::Mul, a, b)
    }

    pub fn div(&mut self, a: T, b: T) -> T {
        self.arith(Op::Div, a, b)
    }

    pub fn modulo(&mut self, a: T, b: T) -> T {
        self.arith(Op::Mod, a, b)
    }

    pub fn neg(&mut self, a: T) -> T {
        let zero = self.int(0);
        self.sub(zero, a)
    }

    /// `a <= b`, folded on constants.
    pub fn le(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_num(a), self.as_num(b)) {
            let v = x <= y;
            return self.bool(v);
        }
        self.app(Op::Le, vec![a, b], Sort::Bool)
    }

    /// `a < b`, folded on constants.
    pub fn lt(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_num(a), self.as_num(b)) {
            let v = x < y;
            return self.bool(v);
        }
        self.app(Op::Lt, vec![a, b], Sort::Bool)
    }

    /// `a = b` of two terms of one sort, folded on constants and on a term equal to itself.
    pub fn eq(&mut self, a: T, b: T) -> T {
        if a == b {
            return self.bool(true);
        }
        let folded = match (self.node(a), self.node(b)) {
            (Node::Num(x), Node::Num(y)) => Some(x == y),
            (Node::Bool(x), Node::Bool(y)) => Some(x == y),
            _ => None,
        };
        if let Some(v) = folded {
            return self.bool(v);
        }
        self.app(Op::Eq, vec![a, b], Sort::Bool)
    }

    pub fn not(&mut self, a: T) -> T {
        if let Some(b) = self.as_bool(a) {
            return self.bool(!b);
        }
        if let Node::App(Op::Not, args) = self.node(a) {
            return args[0];
        }
        self.app(Op::Not, vec![a], Sort::Bool)
    }

    /// The conjunction of `items`: `true` for none, `false` where one is.
    pub fn and(&mut self, items: &[T]) -> T {
        let mut kept = Vec::new();
        for &t in items {
            match self.as_bool(t) {
                Some(false) => return self.bool(false),
                Some(true) => {}
                None => {
                    if !kept.contains(&t) {
                        kept.push(t);
                    }
                }
            }
        }
        match kept.len() {
            0 => self.bool(true),
            1 => kept[0],
            _ => self.app(Op::And, kept, Sort::Bool),
        }
    }

    /// The disjunction of `items`: `false` for none, `true` where one is.
    pub fn or(&mut self, items: &[T]) -> T {
        let mut kept = Vec::new();
        for &t in items {
            match self.as_bool(t) {
                Some(true) => return self.bool(true),
                Some(false) => {}
                None => {
                    if !kept.contains(&t) {
                        kept.push(t);
                    }
                }
            }
        }
        match kept.len() {
            0 => self.bool(false),
            1 => kept[0],
            _ => self.app(Op::Or, kept, Sort::Bool),
        }
    }

    pub fn and2(&mut self, a: T, b: T) -> T {
        self.and(&[a, b])
    }

    pub fn or2(&mut self, a: T, b: T) -> T {
        self.or(&[a, b])
    }

    /// `if c then a else b`, of two terms of one sort.
    pub fn ite(&mut self, c: T, a: T, b: T) -> T {
        if a == b {
            return a;
        }
        match self.as_bool(c) {
            Some(true) => return a,
            Some(false) => return b,
            None => {}
        }
        let sort = self.sort(a).clone();
        self.app(Op::Ite, vec![c, a, b], sort)
    }

    /// The element `i` of the array `a`.
    pub fn select(&mut self, a: T, i: T) -> T {
        let sort = match self.sort(a) {
            Sort::Array(e) => (**e).clone(),
            Sort::Int | Sort::Bool => Sort::Int,
        };
        self.app(Op::Select, vec![a, i], sort)
    }

    /// The array `a` with its element `i` set to `v`.
    pub fn store(&mut self, a: T, i: T, v: T) -> T {
        let sort = self.sort(a).clone();
        self.app(Op::Store, vec![a, i, v], sort)
    }

    /// The array whose every element is `v`.
    pub fn konst(&mut self, v: T) -> T {
        let elem = self.sort(v).clone();
        let sort = Sort::Array(Box::new(elem.clone()));
        self.make(Node::Const(elem, v), sort)
    }

    /// The unsigned value of the bit operation `b` on the `width`-bit two's-complement bits of
    /// `args`: one for `Not`, two for the others.
    pub fn bits(&mut self, b: Bits, width: u32, args: Vec<T>) -> T {
        self.app(Op::Bits(b, width), args, Sort::Int)
    }

    /// The terms `roots` read, at any depth, in the order of the table, children first.
    pub fn reachable(&self, roots: &[T]) -> Vec<T> {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack: Vec<T> = roots.to_vec();
        while let Some(t) = stack.pop() {
            if std::mem::replace(&mut seen[t.0 as usize], true) {
                continue;
            }
            match self.node(t) {
                Node::App(_, args) => stack.extend(args.iter().copied()),
                Node::Const(_, v) => stack.push(*v),
                Node::Num(_) | Node::Bool(_) | Node::Input(..) => {}
            }
        }
        (0..self.nodes.len() as u32)
            .map(T)
            .filter(|t| seen[t.0 as usize])
            .collect()
    }

    /// How a term stands where another reads it: a constant or an input as itself, any other
    /// term by the name of its definition.
    pub fn atom(&self, t: T) -> String {
        match self.node(t) {
            Node::Num(v) => v.smt(),
            Node::Bool(b) => b.to_string(),
            Node::Input(name, _) => name.clone(),
            Node::App(..) | Node::Const(..) => format!("t{}", t.0),
        }
    }

    /// The body of a term's definition.
    fn body(&self, t: T) -> String {
        match self.node(t) {
            Node::App(Op::Bits(b, width), args) => {
                let bv = |x: &T| format!("((_ int2bv {}) {})", width, self.atom(*x));
                let inner: Vec<String> = args.iter().map(bv).collect();
                format!("(bv2nat ({} {}))", b.smt(), inner.join(" "))
            }
            Node::App(op, args) => {
                let inner: Vec<String> = args.iter().map(|a| self.atom(*a)).collect();
                format!("({} {})", op.smt(), inner.join(" "))
            }
            Node::Const(elem, v) => format!(
                "((as const {}) {})",
                Sort::Array(Box::new(elem.clone())).smt(),
                self.atom(*v)
            ),
            Node::Num(_) | Node::Bool(_) | Node::Input(..) => self.atom(t),
        }
    }

    /// The declarations and definitions `roots` need: each input as a `declare-const`, then
    /// each other term they read as a `define-fun`, in order.
    pub fn write(&self, roots: &[T], out: &mut String) {
        let terms = self.reachable(roots);
        for &t in &terms {
            if let Node::Input(name, sort) = self.node(t) {
                let _ = writeln!(out, "(declare-const {} {})", name, sort.smt());
            }
        }
        for &t in &terms {
            match self.node(t) {
                Node::App(..) | Node::Const(..) => {
                    let _ = writeln!(
                        out,
                        "(define-fun t{} () {} {})",
                        t.0,
                        self.sort(t).smt(),
                        self.body(t)
                    );
                }
                Node::Num(_) | Node::Bool(_) | Node::Input(..) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terms_are_made_once_and_folded_on_constants() {
        let mut t = Terms::default();
        assert!(t.is_empty());
        let x = t.input("p.x".into(), Sort::Int);
        assert!(!t.is_empty() && t.len() == 1);
        let one = t.int(1);
        let a = t.add(x, one);
        assert_eq!(t.add(x, one), a);
        let two = t.int(2);
        let three = t.add(one, two);
        assert_eq!(t.as_num(three), Some(&Big::from_i128(3)));
        let m = t.mul(two, three);
        assert_eq!(t.as_num(m), Some(&Big::from_i128(6)));
        let d = t.div(three, two);
        assert_eq!(t.as_num(d), Some(&Big::from_i128(1)));
        let r = t.modulo(three, two);
        assert_eq!(t.as_num(r), Some(&Big::from_i128(1)));
        let s = t.sub(one, three);
        assert_eq!(t.as_num(s), Some(&Big::from_i128(-2)));
        let zero = t.int(0);
        let by_zero = t.div(three, zero);
        assert_eq!(t.as_num(by_zero), None);
        let tru = t.bool(true);
        let fals = t.bool(false);
        let le = t.le(one, two);
        assert_eq!(le, tru);
        let lt = t.lt(two, two);
        assert_eq!(lt, fals);
        assert_eq!(t.le(two, two), tru);
        assert_eq!(t.eq(x, x), tru);
        assert_eq!(t.eq(one, two), fals);
        assert_eq!(t.eq(tru, tru), tru);
        // Two boolean constants that differ fold to false, as two numbers do.
        assert_eq!(t.eq(tru, fals), fals);
        assert_eq!(t.eq(fals, tru), fals);
        let c = t.lt(x, two);
        assert_eq!(t.and(&[tru, c, c]), c);
        assert_eq!(t.and(&[c, fals]), fals);
        assert_eq!(t.and(&[]), tru);
        assert_eq!(t.or(&[fals, c]), c);
        assert_eq!(t.or(&[c, tru]), tru);
        assert_eq!(t.or(&[]), fals);
        let nc = t.not(c);
        assert_eq!(t.not(nc), c);
        assert_eq!(t.not(tru), fals);
        assert_eq!(t.ite(tru, x, one), x);
        assert_eq!(t.ite(fals, x, one), one);
        assert_eq!(t.ite(c, x, x), x);
        let both = t.and(&[c, nc]);
        assert_ne!(both, fals);
        let either = t.or(&[c, nc]);
        assert_ne!(either, tru);
    }

    #[test]
    fn test_a_query_declares_its_inputs_and_defines_each_term_once() {
        let mut t = Terms::default();
        let x = t.input("p.x".into(), Sort::Int);
        let a = t.input("p.a.0".into(), Sort::Int);
        let zero = t.int(0);
        let neg = t.int(-5);
        let arr = t.konst(zero);
        let arr = t.store(arr, zero, a);
        let e = t.select(arr, x);
        let sum = t.add(e, neg);
        let bits = t.bits(Bits::And, 8, vec![sum, x]);
        let not = t.bits(Bits::Not, 8, vec![x]);
        let c = t.lt(bits, not);
        let b = t.input("p.b".into(), Sort::Bool);
        let flag = t.konst(b);
        let f0 = t.select(flag, zero);
        let goal = t.and(&[c, f0]);
        let mut out = String::new();
        t.write(&[goal], &mut out);
        assert_eq!(
            out,
            "(declare-const p.x Int)\n\
             (declare-const p.a.0 Int)\n\
             (declare-const p.b Bool)\n\
             (define-fun t4 () (Array Int Int) ((as const (Array Int Int)) 0))\n\
             (define-fun t5 () (Array Int Int) (store t4 0 p.a.0))\n\
             (define-fun t6 () Int (select t5 p.x))\n\
             (define-fun t7 () Int (+ t6 (- 5)))\n\
             (define-fun t8 () Int (bv2nat (bvand ((_ int2bv 8) t7) ((_ int2bv 8) p.x))))\n\
             (define-fun t9 () Int (bv2nat (bvnot ((_ int2bv 8) p.x))))\n\
             (define-fun t10 () Bool (< t8 t9))\n\
             (define-fun t12 () (Array Int Bool) ((as const (Array Int Bool)) p.b))\n\
             (define-fun t13 () Bool (select t12 0))\n\
             (define-fun t14 () Bool (and t10 t13))\n"
        );
        assert_eq!(t.sort(flag), &Sort::Array(Box::new(Sort::Bool)));
        assert_eq!(t.sort(f0), &Sort::Bool);
        // Only what the roots read.
        let mut part = String::new();
        t.write(&[c], &mut part);
        assert!(!part.contains("p.b") && part.ends_with("(define-fun t10 () Bool (< t8 t9))\n"));
    }
}

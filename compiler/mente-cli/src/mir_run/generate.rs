//! A generator of well-typed programs of M1 and M2's structs, deterministic from a seed, and of
//! arguments for the public functions of any checked module (`docs/design-brief.md`, "The quality system": a
//! generator of well-typed programs, in the manner of Csmith).
//!
//! A program is built to pass every check by construction, as far as the generator can tell:
//! every name is declared once, every binding is read and every `var` assigned, every operation
//! has operands of the kinds it takes and a static range within 128 bits (`E0222`), and a value
//! never lands in a type its whole static range lies outside. Every loop counts a counter, which
//! nothing else assigns, up or down to a constant, so it ends; a function calls only those
//! defined before it, or itself once on a parameter that falls to zero, so every call ends; and
//! a cost bound keeps the work of a call small. What the generator cannot tell, such as a
//! narrowing Tier 1 proves always traps (`E0508`) or a contract Tier 1 cannot prove, the caller
//! discards and counts.
//!
//! Three modes widen the language as the brief asks the generator to: a `Plain` program carries
//! no refinement and no contract; a `Guarded` one also puts a `guard` before an index, a divisor,
//! a shift amount or a narrowing, from which Tier 1 proves checks the lowering leaves out; a
//! `Refined` one adds refined parameters, bindings and results, `requires`, `ensures`, loop
//! `invariant`s and `decreases` measures, written so that Tier 1 can prove them.
//!
//! A program may declare structs before its functions, each holding integers, `bool`s, small
//! arrays and the structs declared before it, so that none contains itself; a `Refined` one
//! gives some fields refinements with literal bounds. Its functions build structs field by
//! field, read fields down through arrays and other structs, store into fields, pass fields as
//! `mut` arguments, and take and give structs as parameters, results and `if` values.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;
use std::panic;

use mente_interp::{Int, Value};
use mente_sema::structs::Structs;
use mente_sema::CheckedModule;
use mente_syntax::diagnostics::Diagnostic;
use mente_syntax::tree::{Bound, Tree, TypeId, TypeKind};
use mente_syntax::Span;

use super::CallArgs;

/// SplitMix64: a small random source, the same on every platform for a seed.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng(seed);
        r.next();
        r
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0 .. n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }

    /// Whether an event of `percent` in a hundred happens.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }

    fn u128(&mut self) -> u128 {
        (u128::from(self.next()) << 64) | u128::from(self.next())
    }
}

/// An integer of magnitude below 2^128, as the static ranges of `core-calculus.md` item 4 need.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Num {
    neg: bool,
    mag: u128,
}

impl Num {
    const ZERO: Num = Num { neg: false, mag: 0 };

    fn new(neg: bool, mag: u128) -> Num {
        Num {
            neg: neg && mag != 0,
            mag,
        }
    }

    fn small(v: i64) -> Num {
        Num::new(v < 0, u128::from(v.unsigned_abs()))
    }

    fn negated(self) -> Num {
        Num::new(!self.neg, self.mag)
    }

    /// The sum, or `None` when its magnitude reaches 2^128.
    fn add(self, o: Num) -> Option<Num> {
        if self.neg == o.neg {
            return Some(Num::new(self.neg, self.mag.checked_add(o.mag)?));
        }
        Some(match self.mag.cmp(&o.mag) {
            Ordering::Less => Num::new(o.neg, o.mag - self.mag),
            _ => Num::new(self.neg, self.mag - o.mag),
        })
    }

    fn sub(self, o: Num) -> Option<Num> {
        self.add(o.negated())
    }

    fn mul(self, o: Num) -> Option<Num> {
        Some(Num::new(self.neg != o.neg, self.mag.checked_mul(o.mag)?))
    }

    /// The value the interpreters take.
    pub fn value(self) -> Value {
        let i = Int::from_u128(self.mag);
        Value::Int(if self.neg { i.negate() } else { i })
    }
}

impl Ord for Num {
    fn cmp(&self, o: &Num) -> Ordering {
        match (self.neg, o.neg) {
            (false, false) => self.mag.cmp(&o.mag),
            (true, true) => o.mag.cmp(&self.mag),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Num {
    fn partial_cmp(&self, o: &Num) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", if self.neg { "-" } else { "" }, self.mag)
    }
}

/// The values an integer expression can take, as `mente_sema::wide` computes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct R {
    lo: Num,
    hi: Num,
}

impl R {
    fn point(n: Num) -> R {
        R { lo: n, hi: n }
    }

    fn within(m: u128, non_negative: bool) -> R {
        R {
            lo: Num::new(!non_negative, if non_negative { 0 } else { m }),
            hi: Num::new(false, m),
        }
    }

    fn magnitude(self) -> u128 {
        self.lo.mag.max(self.hi.mag)
    }

    /// Whether every value lies within `u128`, or every one within `i128`.
    fn fits(self) -> bool {
        const HALF: u128 = 1 << 127;
        !self.lo.neg || (self.lo.mag <= HALF && (self.hi.neg || self.hi.mag < HALF))
    }

    fn hull(self, o: R) -> R {
        R {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
        }
    }

    fn meets(self, o: R) -> bool {
        self.lo <= o.hi && o.lo <= self.hi
    }

    /// The values of both ranges, when there are any.
    fn meet(self, o: R) -> Option<R> {
        let (lo, hi) = (self.lo.max(o.lo), self.hi.min(o.hi));
        (lo <= hi).then_some(R { lo, hi })
    }

    fn holds(self, n: Num) -> bool {
        self.lo <= n && n <= self.hi
    }
}

/// The static range of `a op b` for an arithmetic `op`, as `mente_sema::wide` computes it, or
/// `None` when a bound reaches 2^128.
fn arithmetic(op: Op, a: R, b: R) -> Option<R> {
    match op {
        Op::Add => Some(R {
            lo: a.lo.add(b.lo)?,
            hi: a.hi.add(b.hi)?,
        }),
        Op::Sub => Some(R {
            lo: a.lo.sub(b.hi)?,
            hi: a.hi.sub(b.lo)?,
        }),
        Op::Mul => {
            let c = [
                a.lo.mul(b.lo)?,
                a.lo.mul(b.hi)?,
                a.hi.mul(b.lo)?,
                a.hi.mul(b.hi)?,
            ];
            Some(R {
                lo: *c.iter().min()?,
                hi: *c.iter().max()?,
            })
        }
        Op::Div => Some(R::within(a.magnitude(), !a.lo.neg && !b.lo.neg)),
        _ => Some(R::within(a.magnitude().min(b.magnitude()), !a.lo.neg)),
    }
}

/// The exact value of `a op b` for an arithmetic `op`: `/` truncates toward zero and `%` takes
/// its dividend's sign; `None` for a zero divisor or beyond 128 bits.
fn fold(op: Op, a: Num, b: Num) -> Option<Num> {
    match op {
        Op::Add => a.add(b),
        Op::Sub => a.sub(b),
        Op::Mul => a.mul(b),
        Op::Div => a
            .mag
            .checked_div(b.mag)
            .map(|q| Num::new(a.neg != b.neg, q)),
        _ => a.mag.checked_rem(b.mag).map(|r| Num::new(a.neg, r)),
    }
}

/// The two's-complement bits of `n`, a value of type `t`.
fn bits(t: IntT, n: Num) -> u128 {
    let raw = if n.neg { n.mag.wrapping_neg() } else { n.mag };
    if t.bits == 128 {
        raw
    } else {
        raw & ((1u128 << t.bits) - 1)
    }
}

/// The value of type `t` whose two's-complement bits are `b`.
fn unbits(t: IntT, b: u128) -> Num {
    let b = if t.bits == 128 {
        b
    } else {
        b & ((1u128 << t.bits) - 1)
    };
    let top = 1u128 << (t.bits - 1);
    if t.signed && b & top != 0 {
        let mag = if t.bits == 128 {
            b.wrapping_neg()
        } else {
            (1u128 << t.bits) - b
        };
        Num::new(true, mag)
    } else {
        Num::new(false, b)
    }
}

/// An integer type of M1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntT {
    pub name: &'static str,
    pub signed: bool,
    pub bits: u32,
}

const fn int(name: &'static str, signed: bool, bits: u32) -> IntT {
    IntT { name, signed, bits }
}

pub const INTS: [IntT; 11] = [
    int("u8", false, 8),
    int("u16", false, 16),
    int("u32", false, 32),
    int("u64", false, 64),
    int("u128", false, 128),
    int("usize", false, 64),
    int("i8", true, 8),
    int("i16", true, 16),
    int("i32", true, 32),
    int("i64", true, 64),
    int("i128", true, 128),
];

const U8: IntT = INTS[0];
const USIZE: IntT = INTS[5];
/// The type of an expression of literals alone.
const I64: IntT = INTS[9];

impl IntT {
    pub fn named(name: &str) -> Option<IntT> {
        INTS.iter().copied().find(|t| t.name == name)
    }

    pub fn min(self) -> Num {
        if self.signed {
            Num::new(true, 1u128 << (self.bits - 1))
        } else {
            Num::ZERO
        }
    }

    pub fn max(self) -> Num {
        Num::new(
            false,
            match (self.signed, self.bits) {
                (true, b) => (1u128 << (b - 1)) - 1,
                (false, 128) => u128::MAX,
                (false, b) => (1u128 << b) - 1,
            },
        )
    }

    fn range(self) -> R {
        R {
            lo: self.min(),
            hi: self.max(),
        }
    }

    /// Whether every value of `o` is one of this type.
    fn holds(self, o: IntT) -> bool {
        self.min() <= o.min() && o.max() <= self.max()
    }
}

/// A type of M1, or a struct or an enum the program declares.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Int(IntT),
    Bool,
    Array(u32, Box<Ty>),
    Struct(String),
    Enum(String),
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int(t) => write!(f, "{}", t.name),
            Ty::Bool => write!(f, "bool"),
            Ty::Array(n, e) => write!(f, "[{}]{}", n, e),
            Ty::Struct(name) | Ty::Enum(name) => write!(f, "{}", name),
        }
    }
}

/// The value of an integer literal as the generator writes one: decimal, `0x` or `0b`, with
/// `_` between digits and a type's suffix or none.
fn literal_value(text: &str) -> Option<Num> {
    let digits = match text.rsplit_once('_') {
        Some((digits, suffix)) if INTS.iter().any(|t| t.name == suffix) => digits,
        _ => text,
    };
    let digits = digits.replace('_', "");
    let mag = if let Some(hex) = digits.strip_prefix("0x") {
        u128::from_str_radix(hex, 16).ok()?
    } else if let Some(bin) = digits.strip_prefix("0b") {
        u128::from_str_radix(bin, 2).ok()?
    } else {
        digits.parse().ok()?
    };
    Some(Num::new(false, mag))
}

/// Whether `e` reads a name a pattern binds, which the generator calls `q` and a number.
fn reads_pattern(e: &E) -> bool {
    names(e, &|n| {
        n.strip_prefix('q')
            .is_some_and(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_digit()))
    })
}

/// Whether `e` reads a name that `is` holds of.
fn names(e: &E, is: &dyn Fn(&str) -> bool) -> bool {
    let sub = |x: &E| names(x, is);
    match e {
        E::Name(n) => is(n),
        E::Lit(_) | E::Builtin(..) => false,
        E::Index(a, b) | E::Bin(_, a, b) => sub(a) || sub(b),
        E::Len(x) | E::Field(x, _) | E::Neg(x) | E::BitNot(x) | E::Not(x) | E::Repeat(x, _) => {
            sub(x)
        }
        E::Construct(_, fields) | E::Variant(_, _, fields) => fields.iter().any(|(_, x)| sub(x)),
        E::Call(_, args) => args.iter().any(|(_, x)| sub(x)),
        E::If(c, a, b) => sub(c) || sub(a) || sub(b),
        E::Array(items) => items.iter().any(sub),
        E::Match(s, arms) => sub(s) || arms.iter().any(|(_, x)| sub(x)),
        E::Return(x) => sub(x),
    }
}

/// The integer `e` is, when it is a literal or one negated.
fn constant(e: &E) -> Option<Num> {
    match e {
        E::Lit(text) => literal_value(text),
        E::Neg(x) => match &**x {
            E::Lit(text) => literal_value(text).map(Num::negated),
            _ => None,
        },
        _ => None,
    }
}

/// The range of the integers a value of type `ty` holds, an integer's own or those of an
/// array's elements, through the arrays nested in it; none when it holds no integers.
fn leaf_range(ty: &Ty) -> Option<R> {
    match ty {
        Ty::Int(t) => Some(t.range()),
        Ty::Array(_, inner) => leaf_range(inner),
        Ty::Bool | Ty::Struct(_) | Ty::Enum(_) => None,
    }
}

/// Whether a value of type `from` flows into a place of type `into`: integers into integers,
/// narrowed, arrays of the same lengths element by element, and a struct into itself.
fn flows(from: &Ty, into: &Ty) -> bool {
    match (from, into) {
        (Ty::Int(_), Ty::Int(_)) | (Ty::Bool, Ty::Bool) => true,
        (Ty::Array(n, a), Ty::Array(m, b)) => n == m && flows(a, b),
        (Ty::Struct(a), Ty::Struct(b)) | (Ty::Enum(a), Ty::Enum(b)) => a == b,
        _ => false,
    }
}

/// A struct the program declares.
#[derive(Clone, Debug)]
struct GStruct {
    name: String,
    fields: Vec<GField>,
}

#[derive(Clone, Debug)]
struct GField {
    name: String,
    ty: Ty,
    /// A refinement with literal bounds, in a `Refined` program: of an integer field, or of
    /// each element of an array of integers.
    refine: Option<R>,
    /// Whether the refinement is written with an exclusive end, `lo..hi + 1`.
    exclusive: bool,
    is_pub: bool,
}

/// An enum the program declares: its variants, each with its payload's fields, none for a
/// variant that holds nothing.
#[derive(Clone, Debug)]
struct GEnum {
    name: String,
    variants: Vec<(String, Vec<GField>)>,
}

/// A pattern of a `match` arm, before its bindings are known to be read: a variant, each field
/// with its type and refinement and whether the arm binds it to a name, an integer literal,
/// which has no suffix (`E0221`), a `bool`, or `_`.
#[derive(Clone, Debug)]
enum Pat {
    Variant(String, String, Vec<(GField, bool)>),
    Int(Num),
    Bool(bool),
    Wild,
}

/// A step of a place from its root binding: an index into an array of a length, or a field.
#[derive(Clone, Debug)]
enum PStep {
    Index(u32),
    Field(String),
}

/// A binary operator, by its precedence (`grammar.md`, item 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
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

impl Op {
    fn text(self) -> &'static str {
        match self {
            Op::Or => "or",
            Op::And => "and",
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::BitOr => "|",
            Op::BitXor => "^",
            Op::BitAnd => "&",
            Op::Shl => "<<",
            Op::Shr => ">>",
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Rem => "%",
        }
    }

    fn level(self) -> u8 {
        match self {
            Op::Or => 1,
            Op::And => 2,
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => 4,
            Op::BitOr => 5,
            Op::BitXor => 6,
            Op::BitAnd => 7,
            Op::Shl | Op::Shr => 8,
            Op::Add | Op::Sub => 9,
            Op::Mul | Op::Div | Op::Rem => 10,
        }
    }
}

const NOT: u8 = 3;
const COMPARISON: u8 = 4;
const UNARY: u8 = 11;
const POSTFIX: u8 = 12;
const PRIMARY: u8 = 13;

/// An expression, printed with as few parentheses as the grammar's precedence allows.
#[derive(Clone, Debug)]
enum E {
    Lit(String),
    Name(String),
    Index(Box<E>, Box<E>),
    Len(Box<E>),
    /// A struct's field.
    Field(Box<E>, String),
    /// A struct built with its fields, in the order of its declaration.
    Construct(String, Vec<(String, E)>),
    /// A variant of an enum built, `Name.A` or `Name.B(field: value, ...)`.
    Variant(String, String, Vec<(String, E)>),
    /// A `match` that gives a value: its scrutinee, and each arm's pattern and value.
    Match(Box<E>, Vec<(String, E)>),
    Call(String, Vec<(bool, E)>),
    Neg(Box<E>),
    BitNot(Box<E>),
    Not(Box<E>),
    Bin(Op, Box<E>, Box<E>),
    If(Box<E>, Box<E>, Box<E>),
    Array(Vec<E>),
    Repeat(Box<E>, u32),
    Builtin(&'static str, String),
    /// `return` with a value, as a branch of an `if` or an arm of a `match` in a function's
    /// expression body: the branch leaves the function.
    Return(Box<E>),
}

impl E {
    fn bin(op: Op, a: E, b: E) -> E {
        E::Bin(op, Box::new(a), Box::new(b))
    }

    fn name(n: &str) -> E {
        E::Name(n.to_string())
    }

    fn num(n: Num) -> E {
        let lit = E::Lit(n.mag.to_string());
        if n.neg {
            E::Neg(Box::new(lit))
        } else {
            lit
        }
    }

    fn level(&self) -> u8 {
        match self {
            E::If(..) | E::Match(..) | E::Return(_) => 0,
            E::Bin(op, ..) => op.level(),
            E::Not(_) => NOT,
            E::Neg(_) | E::BitNot(_) => UNARY,
            E::Index(..) | E::Len(_) | E::Field(..) => POSTFIX,
            _ => PRIMARY,
        }
    }

    fn print(&self, min: u8, out: &mut String) {
        if self.level() < min {
            out.push('(');
            self.print(0, out);
            out.push(')');
            return;
        }
        match self {
            E::Lit(s) | E::Name(s) => out.push_str(s),
            E::Index(b, i) => {
                b.print(POSTFIX, out);
                out.push('[');
                i.print(0, out);
                out.push(']');
            }
            E::Len(b) => {
                b.print(POSTFIX, out);
                out.push_str(".len");
            }
            E::Field(b, name) => {
                b.print(POSTFIX, out);
                out.push('.');
                out.push_str(name);
            }
            E::Construct(name, fields) => {
                out.push_str(name);
                out.push('(');
                for (k, (f, v)) in fields.iter().enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(f);
                    out.push_str(": ");
                    v.print(0, out);
                }
                out.push(')');
            }
            E::Variant(name, variant, fields) => {
                out.push_str(name);
                out.push('.');
                out.push_str(variant);
                if !fields.is_empty() {
                    out.push('(');
                    for (k, (f, v)) in fields.iter().enumerate() {
                        if k > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(f);
                        out.push_str(": ");
                        v.print(0, out);
                    }
                    out.push(')');
                }
            }
            // One arm per line: inside the braces a newline ends an arm, wherever it stands.
            E::Match(s, arms) => {
                out.push_str("match ");
                s.print(1, out);
                out.push_str(" {");
                for (p, v) in arms {
                    out.push_str("\n  ");
                    out.push_str(p);
                    out.push_str(" => ");
                    // An arm holds an expression or a block, and `return` is a statement.
                    if let E::Return(_) = v {
                        out.push_str("{ ");
                        v.print(0, out);
                        out.push_str(" }");
                    } else {
                        v.print(0, out);
                    }
                }
                out.push_str("\n}");
            }
            E::Call(f, args) => {
                out.push_str(f);
                out.push('(');
                for (k, (is_mut, a)) in args.iter().enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    if *is_mut {
                        out.push_str("mut ");
                    }
                    a.print(0, out);
                }
                out.push(')');
            }
            E::Neg(x) | E::BitNot(x) => {
                out.push(if matches!(self, E::Neg(_)) { '-' } else { '~' });
                // `- -x` would read as one token to a reader, if not to the lexer.
                let min = match **x {
                    E::Neg(_) | E::BitNot(_) => PRIMARY,
                    _ => UNARY,
                };
                x.print(min, out);
            }
            E::Not(x) => {
                out.push_str("not ");
                x.print(NOT, out);
            }
            E::Bin(op, a, b) => {
                let l = op.level();
                let (left, right) = if l == COMPARISON {
                    (l + 1, l + 1)
                } else {
                    (l, l + 1)
                };
                a.print(left, out);
                out.push(' ');
                out.push_str(op.text());
                out.push(' ');
                b.print(right, out);
            }
            E::If(c, a, b) => {
                out.push_str("if ");
                c.print(1, out);
                out.push_str(" { ");
                a.print(0, out);
                out.push_str(" } else { ");
                b.print(0, out);
                out.push_str(" }");
            }
            E::Array(items) => {
                out.push('[');
                for (k, e) in items.iter().enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    e.print(0, out);
                }
                out.push(']');
            }
            E::Repeat(v, n) => {
                out.push('[');
                v.print(0, out);
                out.push_str(&format!("; {}]", n));
            }
            E::Builtin(f, reason) => out.push_str(&format!("{}(\"{}\")", f, reason)),
            E::Return(v) => {
                out.push_str("return ");
                v.print(0, out);
            }
        }
    }

    fn text(&self) -> String {
        let mut s = String::new();
        self.print(0, &mut s);
        s
    }

    /// Its text where a statement's header holds it, before the `{` of a block: the condition
    /// of an `if`, a `while` or a `guard`, and what a `match` reads. An `if` or a `match` there
    /// stands in parentheses, as it does as an operand (`E0115`).
    fn header(&self) -> String {
        let mut s = String::new();
        self.print(1, &mut s);
        s
    }
}

/// What the operands of an integer expression have in common, which decides its type
/// (`core-calculus.md` item 5): one integer type, literals alone, or several types, with the
/// smallest standard type that holds them all, if there is one, as the checker keeps it
/// (`types.rs`, `Int`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cls {
    One(IntT),
    Lit,
    Mixed(Option<IntT>),
}

/// The standard types from the smallest: the order in which the checker looks for the type
/// that holds two others.
const BY_SIZE: [IntT; 11] = [
    INTS[0], INTS[6], INTS[1], INTS[7], INTS[2], INTS[8], INTS[3], INTS[5], INTS[9], INTS[4],
    INTS[10],
];

impl Cls {
    fn join(self, o: Cls) -> Cls {
        match (self, o) {
            (Cls::Lit, x) | (x, Cls::Lit) => x,
            (Cls::One(a), Cls::One(b)) if a == b => Cls::One(a),
            (a, b) => Cls::Mixed(a.holder().zip(b.holder()).and_then(|(a, b)| {
                if a.holds(b) {
                    Some(a)
                } else if b.holds(a) {
                    Some(b)
                } else {
                    BY_SIZE.iter().copied().find(|t| t.holds(a) && t.holds(b))
                }
            })),
        }
    }

    /// The smallest standard type that holds this class's values, if there is one.
    fn holder(self) -> Option<IntT> {
        match self {
            Cls::Lit => Some(I64),
            Cls::One(t) => Some(t),
            Cls::Mixed(t) => t,
        }
    }

    /// The 128-bit type that makes an operation on this operand checked at the operator: its
    /// one type, or the type that holds its several.
    fn wide(self) -> Option<IntT> {
        match self {
            Cls::One(t) | Cls::Mixed(Some(t)) if t.bits == 128 => Some(t),
            _ => None,
        }
    }
}

/// An integer expression, with its operands' type and its static range.
#[derive(Clone, Debug)]
struct G {
    e: E,
    cls: Cls,
    r: R,
    /// Its value, when it reads no binding and the generator can tell it.
    k: Option<Num>,
}

/// What an integer expression must be: any integer, or one whose operands all have one type,
/// as a bit operation's, a shift's left operand and `~`'s need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Any,
    Exact(IntT),
}

/// How much of M1 a program uses (see the module's documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Plain,
    Guarded,
    Refined,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Plain, Mode::Guarded, Mode::Refined];

    /// The mode of the program a seed generates: the three in turn.
    pub fn of(seed: u64) -> Mode {
        Mode::ALL[(seed % 3) as usize]
    }
}

/// A generated program.
#[derive(Clone, Debug)]
pub struct Program {
    pub seed: u64,
    pub mode: Mode,
    pub text: String,
    /// Each function's name, with where its text lies in `text`.
    pub functions: Vec<(String, Range<usize>)>,
}

/// The program `seed` generates, the same on every platform.
pub fn program(seed: u64) -> Program {
    generated(seed, &[])
}

/// The program of `seed` whose function `k` has the body of its attempt `attempts[k]`, the
/// first one where none is given: every attempt keeps the function's signature.
fn generated(seed: u64, attempts: &[u32]) -> Program {
    let mode = Mode::of(seed);
    let mut g = Gen::new(seed, mode);
    g.attempts = attempts.to_vec();
    let (text, functions) = g.module();
    Program {
        seed,
        mode,
        text,
        functions,
    }
}

/// The random source of function `index` of the program of `seed`: `part` 0 for its signature,
/// which every attempt shares, and `1 + attempt` for its body.
fn stream(seed: u64, index: usize, part: u64) -> Rng {
    let mut r = Rng::new(seed ^ 0xD6E8_FEB8_6659_FD93_u64.wrapping_mul(index as u64 + 1));
    r.0 ^= part.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    r.next();
    r
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Val,
    Var,
    Param,
    MutParam,
    /// A loop's counter, which only the loop's first statement changes.
    Counter,
    /// A name a `match` pattern binds to a field of a variant's payload, which nothing changes.
    Pattern,
}

#[derive(Clone, Debug)]
struct Binding {
    name: String,
    ty: Ty,
    kind: Kind,
    /// A refinement with literal bounds, which is the range Tier 1 and `E0222` take for it.
    range: Option<R>,
    /// What a run can know of an integer's value where generation stands, as Tier 1 follows
    /// it: its initializer's range, an assignment's, and what a `guard` or a loop leaves of
    /// it. None for no more than its type and refinement say. Only what a narrowing may take
    /// reads it ([`Gen::fact`]): nothing that must be proven rests on it.
    known: Option<R>,
    /// The refinement of its elements, with literal bounds, when a pattern binds it to a field
    /// that is an array of refined integers: a `match` on one of its elements covers that
    /// range (`core-calculus.md`, item 14).
    elements: Option<R>,
    read: bool,
    assigned: bool,
}

/// A refinement bound of a parameter.
#[derive(Clone, Debug)]
enum Bnd {
    Lit(Num),
    Name(usize),
    Len(usize),
}

#[derive(Clone, Debug)]
struct Refine {
    lo: Bnd,
    hi: Bnd,
    inclusive: bool,
}

#[derive(Clone, Debug)]
struct Param {
    name: String,
    is_mut: bool,
    ty: Ty,
    refine: Option<Refine>,
}

/// A `requires` clause over parameters: `a op c`, `a op b`, `a + b <= c` or `a < b.len`.
#[derive(Clone, Debug)]
enum Req {
    /// `a op c`.
    Bound(usize, Op, Num),
    /// `a op b`.
    Order(usize, Op, usize),
    /// `a + b <= c`.
    Sum(usize, usize, Num),
    /// `a < b.len`.
    Below(usize, usize),
}

impl Sig {
    /// For each parameter, the parameters a `requires` or a refinement bound relates it to:
    /// `a op b`, `a + b <= c`, and a bound that names another parameter.
    fn related(&self) -> Vec<Vec<usize>> {
        let mut out = vec![Vec::new(); self.params.len()];
        let mut pair = |a: usize, b: usize| {
            out[a].push(b);
            out[b].push(a);
        };
        for r in &self.requires {
            match *r {
                Req::Order(a, _, b) | Req::Sum(a, b, _) => pair(a, b),
                Req::Bound(..) | Req::Below(..) => {}
            }
        }
        for (k, p) in self.params.iter().enumerate() {
            for b in p.refine.iter().flat_map(|r| [&r.lo, &r.hi]) {
                match b {
                    Bnd::Name(i) => pair(k, *i),
                    Bnd::Lit(_) | Bnd::Len(_) => {}
                }
            }
        }
        out
    }
}

/// A function's signature as its callers see it, with the work a call does at most.
#[derive(Clone, Debug)]
struct Sig {
    name: String,
    params: Vec<Param>,
    ret: Option<Ty>,
    /// The literal range of its result, from a refinement or `ensures`.
    ret_range: Option<R>,
    requires: Vec<Req>,
    cost: u64,
}

#[derive(Clone, Debug)]
enum S {
    /// A binding, with the type it states, or none when it takes its initializer's.
    Bind(usize, Option<String>, E),
    Assign(E, &'static str, E),
    Discard(E),
    Expr(E),
    If(E, Block, Option<Block>),
    /// A `match` statement: its scrutinee, and each arm's pattern and block.
    Match(E, Vec<(String, Block)>),
    While(E, Vec<String>, Block),
    Guard(E, Block),
    Return(Option<E>),
    Break,
    Continue,
}

impl S {
    /// Whether control never goes past it.
    fn leaves(&self) -> bool {
        matches!(
            self,
            S::Return(_) | S::Break | S::Continue | S::Expr(E::Builtin(..))
        )
    }
}

#[derive(Clone, Debug, Default)]
struct Block {
    stmts: Vec<S>,
    tail: Option<E>,
}

/// A function's body: `= expression` or a block.
enum Body {
    Expr(E),
    Block(Block),
}

/// The most work, in statements run, one call of a generated function may do.
const COST_BUDGET: u64 = 4_000;

/// What `Gen::mark` saves.
struct Mark {
    seen: Vec<(bool, bool)>,
    pending: usize,
    cost: u64,
    recursed: bool,
}

struct Gen {
    rng: Rng,
    mode: Mode,
    /// The structs declared so far.
    structs: Vec<GStruct>,
    /// The enums declared so far.
    enums: Vec<GEnum>,
    /// The roots of the places a `match` reads, which nothing in any of its arms changes
    /// (`core-calculus.md`, item 14).
    frozen: Vec<usize>,
    /// Whether an integer expression being generated divides by a divisor known to be zero.
    zero_divisor: bool,
    /// The program's seed, which with a function's index gives the function its random sources.
    seed: u64,
    /// Which attempt at its body each function takes, the first where none is given.
    attempts: Vec<u32>,
    /// The range of the integers each array literal being generated holds so far, the innermost
    /// last: those of the literals nested in one another are kept in one machine type together
    /// (`E0222`). A value that is no literal starts one of its own, which nothing reads.
    hulls: Vec<Option<R>>,
    sigs: Vec<Sig>,
    /// The function being generated, which may call itself once when it has a measure.
    current: Option<Sig>,
    /// The depth bound of the function's recursion, when it recurses.
    recursion: Option<u64>,
    /// Whether the function being generated is written to stay small ([`Gen::relates`]).
    small: bool,
    recursed: bool,
    binds: Vec<Binding>,
    scopes: Vec<Vec<usize>>,
    /// Bindings no expression may read now: the roots of a call's `mut` arguments.
    exclude: Vec<usize>,
    /// Conditions to `guard` before the statement being generated.
    pending: Vec<E>,
    /// Index expressions that stay below a bound: loop counters inside their loops, and
    /// parameters a `requires` bounds.
    safe: Vec<(E, u128)>,
    loops: u32,
    mult: u64,
    cost: u64,
    no_calls: bool,
    names: usize,
}

impl Gen {
    fn new(seed: u64, mode: Mode) -> Gen {
        Gen {
            rng: Rng::new(seed),
            seed,
            attempts: Vec::new(),
            zero_divisor: false,
            mode,
            structs: Vec::new(),
            enums: Vec::new(),
            frozen: Vec::new(),
            hulls: Vec::new(),
            sigs: Vec::new(),
            current: None,
            recursion: None,
            small: false,
            recursed: false,
            binds: Vec::new(),
            scopes: Vec::new(),
            exclude: Vec::new(),
            pending: Vec::new(),
            safe: Vec::new(),
            loops: 0,
            mult: 1,
            cost: 0,
            no_calls: false,
            names: 0,
        }
    }

    fn module(&mut self) -> (String, Vec<(String, Range<usize>)>) {
        let mut out = String::from("module generated\n");
        // Structs and enums first, each holding only those declared before it, so none contains
        // itself.
        if self.rng.chance(70) {
            for _ in 0..1 + self.rng.below(4) as usize {
                let text = if self.rng.chance(45) {
                    self.enum_decl(self.enums.len())
                } else {
                    self.struct_decl(self.structs.len())
                };
                out.push('\n');
                out.push_str(&text);
            }
        }
        // What the module decides of its functions comes first: each function then has random
        // sources of its own, so that one can be generated again and leave the others as they
        // are.
        let count = 1 + self.rng.below(5) as usize;
        let public: Vec<bool> = (0..count)
            .map(|k| k + 1 == count || self.rng.chance(50))
            .collect();
        let mut functions = Vec::new();
        for (k, public) in public.into_iter().enumerate() {
            let text = self.function(k, public);
            out.push('\n');
            let start = out.len();
            out.push_str(&text);
            functions.push((format!("f{}", k), start..out.len()));
        }
        (out, functions)
    }

    // ---------------------------------------------------------------- structs

    /// A struct of one to four fields, and its declaration's text.
    fn struct_decl(&mut self, k: usize) -> String {
        let name = format!("S{}", k);
        let mut fields = Vec::new();
        for j in 0..1 + self.rng.below(4) as usize {
            let f = self.field(format!("x{}", j));
            fields.push(f);
        }
        let mut out = String::new();
        if self.rng.chance(50) {
            out.push_str("pub ");
        }
        out.push_str(&format!("struct {} {{\n", name));
        for f in &fields {
            let p = if f.is_pub { "pub " } else { "" };
            out.push_str(&format!("  {}{}\n", p, field_text(f)));
        }
        out.push_str("}\n");
        self.structs.push(GStruct { name, fields });
        out
    }

    /// An enum of one to five variants, each holding nothing or a payload of one to three
    /// fields, and its declaration's text.
    fn enum_decl(&mut self, k: usize) -> String {
        let name = format!("E{}", k);
        let mut variants = Vec::new();
        for j in 0..1 + self.rng.below(5) as usize {
            let mut fields = Vec::new();
            if self.rng.chance(55) {
                for i in 0..1 + self.rng.below(3) as usize {
                    let mut f = self.field(format!("y{}", i));
                    f.is_pub = false;
                    fields.push(f);
                }
            }
            variants.push((format!("V{}", j), fields));
        }
        let mut out = String::new();
        if self.rng.chance(50) {
            out.push_str("pub ");
        }
        out.push_str(&format!("enum {} {{\n", name));
        for (v, fields) in &variants {
            if fields.is_empty() {
                out.push_str(&format!("  {}\n", v));
            } else {
                let payload: Vec<String> = fields.iter().map(field_text).collect();
                out.push_str(&format!("  {}({})\n", v, payload.join(", ")));
            }
        }
        out.push_str("}\n");
        self.enums.push(GEnum { name, variants });
        out
    }

    /// A field of a struct or of a payload: its type, and in a `Refined` program now and then a
    /// refinement of an integer, or of each element of an array of integers.
    fn field(&mut self, name: String) -> GField {
        let ty = self.field_ty();
        let int = match &ty {
            Ty::Int(t) => Some(*t),
            Ty::Array(_, e) => match **e {
                Ty::Int(t) => Some(t),
                _ => None,
            },
            _ => None,
        };
        let refine = match int {
            Some(t) if self.mode == Mode::Refined && self.rng.chance(40) => {
                let lo = self.value_in(R {
                    lo: t.min().max(Num::small(-50)),
                    hi: t.max().min(Num::small(50)),
                });
                let width = Num::new(false, u128::from(self.rng.below(100)));
                Some(R {
                    lo,
                    hi: lo.add(width).map_or(t.max(), |h| h.min(t.max())),
                })
            }
            _ => None,
        };
        // An exclusive end where one past the last value fits the type.
        let exclusive = match (int, refine) {
            (Some(t), Some(r)) => r.hi < t.max() && self.rng.chance(50),
            _ => false,
        };
        GField {
            name,
            ty,
            refine,
            exclusive,
            is_pub: self.rng.chance(30),
        }
    }

    /// A field's type: an integer, a `bool`, a small array, or a struct or an enum declared
    /// before.
    fn field_ty(&mut self) -> Ty {
        let structs = self.structs.len() as u64;
        let enums = self.enums.len() as u64;
        match self.rng.below(100) {
            0..=54 => Ty::Int(self.int_ty()),
            55..=66 => Ty::Bool,
            67..=79 => {
                let n = 1 + self.rng.below(4) as u32;
                let elem = if self.rng.chance(70) {
                    Ty::Int(self.int_ty())
                } else {
                    Ty::Bool
                };
                Ty::Array(n, Box::new(elem))
            }
            80..=84 if structs > 0 => {
                let k = self.rng.below(structs) as usize;
                let n = 1 + self.rng.below(3) as u32;
                Ty::Array(n, Box::new(Ty::Struct(self.structs[k].name.clone())))
            }
            85..=91 if enums > 0 => {
                let k = self.rng.below(enums) as usize;
                Ty::Enum(self.enums[k].name.clone())
            }
            _ if structs > 0 => {
                let k = self.rng.below(structs) as usize;
                Ty::Struct(self.structs[k].name.clone())
            }
            _ => Ty::Int(self.int_ty()),
        }
    }

    /// The variants of enum `name`, each with its payload's fields.
    fn variants_of(&self, name: &str) -> Vec<(String, Vec<GField>)> {
        self.enums
            .iter()
            .find(|e| e.name == name)
            .map_or_else(Vec::new, |e| e.variants.clone())
    }

    /// A variant of enum `name` built, its payload's values each landing in its field's type, a
    /// refined field's inside its refinement.
    fn variant(&mut self, name: &str, depth: u32) -> E {
        let variants = self.variants_of(name);
        let (v, fields) = self.rng.pick(&variants).clone();
        let mut values = Vec::new();
        for f in fields {
            let x = match (&f.ty, f.refine) {
                (Ty::Int(_), Some(r)) => self.refined_value(r),
                (Ty::Array(n, _), Some(r)) => self.refined_array(*n, r),
                (ty, _) => self.value(ty, depth + 1),
            };
            values.push((f.name, x));
        }
        E::Variant(name.to_string(), v, values)
    }

    /// A value of enum `name`: a binding's, a call's, a part of another value, or a new one.
    fn enum_value(&mut self, name: &str, depth: u32) -> E {
        let want = Ty::Enum(name.to_string());
        match self.rng.below(100) {
            0..=29 => match self.part(&|t: &Ty| *t == want, false) {
                Some((e, _)) => e,
                None => self.variant(name, depth),
            },
            30..=44 => match self.call(depth, |t| *t == want) {
                Some((e, _)) => e,
                None => self.variant(name, depth),
            },
            _ => self.variant(name, depth),
        }
    }

    // ---------------------------------------------------------------- `match`

    /// The patterns of a `match` on a value of `ty`, exhaustive and each one reached: every
    /// variant of an enum once, in some order, binding some of its payload's fields; `true` and
    /// `false`; or distinct literals of an integer's `range`, its type's or its refinement's,
    /// then `_` unless they cover it; an 8-bit type is covered whole `whole` times in a
    /// hundred. `None` for a type a `match` does not read.
    fn patterns(&mut self, ty: &Ty, range: Option<R>, whole: u64) -> Option<Vec<Pat>> {
        match ty {
            Ty::Enum(name) => {
                let mut variants = self.variants_of(name);
                if self.rng.chance(40) {
                    variants.reverse();
                }
                Some(
                    variants
                        .into_iter()
                        .map(|(v, fields)| {
                            let fields = fields
                                .into_iter()
                                .map(|f| (f, self.rng.chance(70)))
                                .collect();
                            Pat::Variant(name.clone(), v, fields)
                        })
                        .collect(),
                )
            }
            Ty::Bool => {
                let first = self.rng.chance(50);
                Some(vec![Pat::Bool(first), Pat::Bool(!first)])
            }
            Ty::Int(_) => {
                let r = range?;
                let span = r.hi.sub(r.lo).map_or(u128::MAX, |s| s.mag);
                // A small range covered by its literals alone, now and then, and an 8-bit type
                // too, which a scrutinee is narrowed into ([`match-scrutinee-narrowing`]).
                if (span < 4 && self.rng.chance(60))
                    || (span == 255 && !self.relates() && self.rng.chance(whole))
                {
                    let mut out = Vec::new();
                    let mut n = r.lo;
                    while n <= r.hi {
                        out.push(Pat::Int(n));
                        n = n.add(Num::small(1))?;
                    }
                    if self.rng.chance(30) {
                        out.reverse();
                    }
                    return Some(out);
                }
                let mut seen: Vec<Num> = Vec::new();
                for _ in 0..1 + self.rng.below(4) {
                    let n = match self.rng.below(10) {
                        0 => r.lo,
                        1 => r.hi,
                        _ => self.value_in(r),
                    };
                    if !seen.contains(&n) {
                        seen.push(n);
                    }
                }
                // `_` stands only where the literals leave a value for it (`E0230`).
                let covered = u128::try_from(seen.len()).is_ok_and(|n| n > span);
                let mut out: Vec<Pat> = seen.into_iter().map(Pat::Int).collect();
                if !covered {
                    out.push(Pat::Wild);
                }
                Some(out)
            }
            Ty::Array(..) | Ty::Struct(_) => None,
        }
    }

    /// A scrutinee of type `ty` and the range of integers its patterns take: a value of an enum
    /// or of `bool`, or an integer of one type, a refined binding's name now and then, whose
    /// refinement bounds the literals.
    fn scrutinee(&mut self, ty: &Ty, depth: u32) -> Option<(E, Option<R>)> {
        // Now and then what a call that changes a place gives, which the arms then read as the
        // call left it. A call's result has its type's range, refined or not.
        if self.rng.chance(60) {
            if let Some((e, _)) = self.call_of(depth + 1, |t| t == ty, true) {
                let range = match ty {
                    Ty::Int(t) => Some(t.range()),
                    Ty::Bool | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) => None,
                };
                return Some((e, range));
            }
        }
        match ty {
            Ty::Enum(name) => Some((self.enum_value(name, depth + 1), None)),
            Ty::Bool => Some((self.bool_expr(depth + 1), None)),
            Ty::Int(t) => {
                // A binding whose refinement has literal bounds: the literals cover its range.
                let refined: Vec<usize> = self
                    .visible()
                    .into_iter()
                    .filter(|&id| {
                        matches!(self.binds[id].ty, Ty::Int(_)) && self.binds[id].range.is_some()
                    })
                    .collect();
                if !refined.is_empty() && self.rng.chance(50) {
                    let id = *self.rng.pick(&refined);
                    let r = self.binds[id].range?;
                    return Some((self.read(id), Some(r)));
                }
                // An arithmetic scrutinee is narrowed into its type, which Tier 1 must prove
                // ([`match-scrutinee-narrowing`]): one whose static range lies within the type,
                // or of a 128-bit type, checked at its operator; now and then an `if` of two
                // such; tried a few times, or a binding of the type read.
                if self.rng.chance(20) {
                    let m = self.mark();
                    let c = self.bool_expr(depth + 1);
                    match (
                        self.int_scrutinee(*t, depth + 1),
                        self.int_scrutinee(*t, depth + 1),
                    ) {
                        (Some((a, ua)), Some((b, ub))) if ua == ub => {
                            let e = E::If(Box::new(c), Box::new(a), Box::new(b));
                            return Some((e, Some(ua.range())));
                        }
                        _ => self.restore(m),
                    }
                }
                if let Some((e, u)) = self.int_scrutinee(*t, depth + 1) {
                    let r = self.declared_range(&e).unwrap_or(u.range());
                    return Some((e, Some(r)));
                }
                let names: Vec<usize> = self
                    .visible()
                    .into_iter()
                    .filter(|&id| self.binds[id].ty == Ty::Int(*t))
                    .collect();
                if names.is_empty() {
                    return None;
                }
                let id = *self.rng.pick(&names);
                let e = self.read(id);
                let r = self.declared_range(&e).unwrap_or(t.range());
                Some((e, Some(r)))
            }
            Ty::Array(..) | Ty::Struct(_) => None,
        }
    }

    /// The refinement the checker reads `e` with, when `e` is a place that declares one with
    /// literal bounds: a binding's, a field's, or an element's of a field whose elements are
    /// refined. A `match` on such a place covers that range, not its type's
    /// (`core-calculus.md`, item 14).
    fn declared_range(&self, e: &E) -> Option<R> {
        let id = self.root(e)?;
        match e {
            E::Name(_) => self.binds[id].range,
            _ => self.place_refine(id, e),
        }
    }

    /// An integer expression of type `t`, or of literals alone, that a `match` can read: one
    /// whose static range lies within its type, or of a 128-bit type, which its operators check
    /// (`core-calculus.md`, item 4); and its type. Tried a few times.
    fn int_scrutinee(&mut self, t: IntT, depth: u32) -> Option<(E, IntT)> {
        for _ in 0..6 {
            let m = self.mark();
            self.zero_divisor = false;
            let g = self.int_expr(Want::Exact(t), depth);
            let u = match g.cls {
                Cls::One(u) => u,
                Cls::Lit => I64,
                Cls::Mixed(_) => {
                    self.restore(m);
                    continue;
                }
            };
            let range = u.range();
            // A division by a divisor known to be zero always traps, and Tier 1 knows of its
            // quotient only that it is no larger than the dividend, of either sign: not that it
            // lies within the type, which a `match` needs proven.
            if (u.bits == 128 || (range.lo <= g.r.lo && g.r.hi <= range.hi)) && !self.zero_divisor {
                return Some((g.e, u));
            }
            self.restore(m);
        }
        None
    }

    /// The type a `match` reads: an enum when the program declares one, else a `bool` or an
    /// integer.
    fn read_ty(&mut self) -> Ty {
        // Now and then the type a function with a `mut` parameter gives, so that a call of it
        // can be what the `match` reads.
        if self.rng.chance(15) {
            let given: Vec<Ty> = self
                .sigs
                .iter()
                .filter(|s| s.params.iter().any(|p| p.is_mut))
                .filter_map(|s| s.ret.clone())
                .filter(|t| matches!(t, Ty::Enum(_) | Ty::Bool | Ty::Int(_)))
                .collect();
            if !given.is_empty() {
                return self.rng.pick(&given).clone();
            }
        }
        let roll = self.rng.below(100);
        if !self.enums.is_empty() && roll < 55 {
            let k = self.rng.below(self.enums.len() as u64) as usize;
            return Ty::Enum(self.enums[k].name.clone());
        }
        if roll < 75 {
            Ty::Bool
        } else {
            Ty::Int(self.int_ty())
        }
    }

    /// The binding at the root of a place `e` names, if it is one.
    fn root(&self, e: &E) -> Option<usize> {
        match e {
            E::Name(n) => self
                .visible()
                .into_iter()
                .rev()
                .find(|&id| self.binds[id].name == *n),
            // An array's length is read from the array, which a `match` on it then freezes.
            E::Index(b, _) | E::Field(b, _) | E::Len(b) => self.root(b),
            _ => None,
        }
    }

    /// Declares the names pattern `p` binds, in the arm's scope, which the caller pushed: each
    /// a read-only binding of its field's type and refinement.
    fn bind_pattern(&mut self, p: &Pat) -> Vec<Option<usize>> {
        let Pat::Variant(_, _, fields) = p else {
            return Vec::new();
        };
        fields
            .iter()
            .map(|(f, binds)| {
                binds.then(|| {
                    let name = self.fresh("q");
                    let (range, elements) = match (&f.ty, f.refine) {
                        (Ty::Int(_), r) => (r, None),
                        (Ty::Array(_, inner), r) if matches!(**inner, Ty::Int(_)) => (None, r),
                        _ => (None, None),
                    };
                    let id = self.declare(name, f.ty.clone(), Kind::Pattern, range);
                    self.binds[id].elements = elements;
                    id
                })
            })
            .collect()
    }

    /// A pattern's text, once its arm is generated: a field whose binding nobody read is `_`.
    fn pattern_text(&self, p: &Pat, ids: &[Option<usize>]) -> String {
        match p {
            Pat::Variant(e, v, fields) => {
                if fields.is_empty() {
                    return format!("{}.{}", e, v);
                }
                let parts: Vec<String> = fields
                    .iter()
                    .zip(ids)
                    .map(|((f, _), id)| match id {
                        Some(id) if self.binds[*id].read => {
                            format!("{}: {}", f.name, self.binds[*id].name)
                        }
                        _ => format!("{}: _", f.name),
                    })
                    .collect();
                format!("{}.{}({})", e, v, parts.join(", "))
            }
            Pat::Int(n) => {
                if n.neg {
                    format!("-{}", n.mag)
                } else {
                    format!("{}", n.mag)
                }
            }
            Pat::Bool(b) => b.to_string(),
            Pat::Wild => String::from("_"),
        }
    }

    /// Whether `e` reads a name of `names`.
    fn reads_any(e: &E, names: &[String]) -> bool {
        let sub = |x: &E| Gen::reads_any(x, names);
        match e {
            E::Name(n) => names.contains(n),
            E::Lit(_) | E::Builtin(..) => false,
            E::Index(a, b) | E::Bin(_, a, b) => sub(a) || sub(b),
            E::Len(x) | E::Field(x, _) | E::Neg(x) | E::BitNot(x) | E::Not(x) | E::Repeat(x, _) => {
                sub(x)
            }
            E::Construct(_, fields) | E::Variant(_, _, fields) => {
                fields.iter().any(|(_, x)| sub(x))
            }
            E::Call(_, args) => args.iter().any(|(_, x)| sub(x)),
            E::If(c, a, b) => sub(c) || sub(a) || sub(b),
            E::Array(items) => items.iter().any(sub),
            E::Match(s, arms) => sub(s) || arms.iter().any(|(_, x)| sub(x)),
            E::Return(x) => sub(x),
        }
    }

    /// Makes, now and then, one branch of an `if` or one arm of a `match` in the expression
    /// body `e` leave the function by `return`, with a value of its result type `t`: an arm
    /// that binds no name, and another branch or arm still gives a value. Now and then every
    /// branch or arm leaves, and the `if` or `match` gives no value, wherever it stands: an
    /// operand, an argument, an element or the body itself (`core-calculus.md`, item 6).
    fn returns_in(&mut self, e: &mut E, t: &Ty) {
        match e {
            E::If(c, a, b) => {
                self.returns_in(c, t);
                // A branch that reads a pattern's binding stays: its pattern is written already,
                // and would bind a name nothing reads any more (`E0218`).
                let free = (!reads_pattern(a), !reads_pattern(b));
                if (free.0 || free.1) && self.rng.chance(25) {
                    let v = E::Return(Box::new(self.value(t, 1)));
                    if free.0 && free.1 && self.rng.chance(30) {
                        **a = v;
                        **b = E::Return(Box::new(self.value(t, 1)));
                    } else if free.0 && (!free.1 || self.rng.chance(50)) {
                        **a = v;
                    } else {
                        **b = v;
                    }
                    return;
                }
                self.returns_in(a, t);
                self.returns_in(b, t);
            }
            E::Match(s, arms) => {
                self.returns_in(s, t);
                let free: Vec<usize> = (0..arms.len())
                    .filter(|&k| binds_nothing(&arms[k].0) && !reads_pattern(&arms[k].1))
                    .collect();
                if arms.len() > 1 && !free.is_empty() && self.rng.chance(25) {
                    if free.len() == arms.len() && self.rng.chance(30) {
                        for arm in arms.iter_mut() {
                            arm.1 = E::Return(Box::new(self.value(t, 1)));
                        }
                    } else {
                        let k = *self.rng.pick(&free);
                        arms[k].1 = E::Return(Box::new(self.value(t, 1)));
                    }
                    return;
                }
                for (_, x) in arms.iter_mut() {
                    self.returns_in(x, t);
                }
            }
            E::Index(a, b) | E::Bin(_, a, b) => {
                self.returns_in(a, t);
                self.returns_in(b, t);
            }
            E::Len(x) | E::Field(x, _) | E::Neg(x) | E::BitNot(x) | E::Not(x) | E::Repeat(x, _) => {
                self.returns_in(x, t)
            }
            E::Construct(_, fields) | E::Variant(_, _, fields) => {
                for (_, x) in fields.iter_mut() {
                    self.returns_in(x, t);
                }
            }
            E::Call(_, args) => {
                for (_, x) in args.iter_mut() {
                    self.returns_in(x, t);
                }
            }
            E::Array(items) => {
                for x in items.iter_mut() {
                    self.returns_in(x, t);
                }
            }
            E::Lit(_) | E::Name(_) | E::Builtin(..) | E::Return(_) => {}
        }
    }

    /// A `match` that gives a value of `ty`, its arms expressions; an integer `match` takes the
    /// hull of its arms' ranges, which must lie within one 128-bit type (`E0222`). An arm whose
    /// value needs a `guard` on a name its pattern binds, which only a block could hold, gives
    /// a default value instead. `None` when no such `match` comes of it.
    fn match_value(&mut self, ty: &Ty, depth: u32) -> Option<(E, Option<R>)> {
        // Now and then an 8-bit type, which literals alone can cover: here each arm is one
        // value, where a statement's 256 arms would each be a block.
        let read = if self.rng.chance(12) {
            Ty::Int(*self.rng.pick(&[INTS[0], INTS[6]]))
        } else {
            self.read_ty()
        };
        let (s, range) = self.scrutinee(&read, depth)?;
        let pats = self.patterns(&read, range, 75)?;
        let root = self.root(&s);
        let mut arms = Vec::new();
        let mut hull: Option<R> = None;
        for p in &pats {
            self.scopes.push(Vec::new());
            let ids = self.bind_pattern(p);
            let frozen = self.frozen.len();
            if let Some(r) = root {
                self.frozen.push(r);
            }
            let names: Vec<String> = ids
                .iter()
                .flatten()
                .map(|&id| self.binds[id].name.clone())
                .collect();
            let m = self.mark();
            let (mut v, mut r) = match ty {
                // An integer arm whose static range the generator knows, for the hull.
                Ty::Int(t) => {
                    let (e, r) = self.narrowed_range(*t, depth + 1);
                    (e, Some(r))
                }
                _ => (self.full(ty, depth + 1), None),
            };
            if self.pending[m.pending..]
                .iter()
                .any(|c| Gen::reads_any(c, &names))
            {
                self.restore(m);
                v = self.default_of(ty, None);
                r = match ty {
                    Ty::Int(_) => Some(R::point(Num::ZERO)),
                    _ => None,
                };
            }
            self.frozen.truncate(frozen);
            self.scopes.pop();
            if let Some(r) = r {
                hull = Some(hull.map_or(r, |h| h.hull(r)));
            }
            arms.push((self.pattern_text(p, &ids), v));
        }
        if hull.is_some_and(|h| !h.fits()) {
            return None;
        }
        Some((E::Match(Box::new(s), arms), hull))
    }

    /// A binding of a small refined range, `val q: u8[0..k] = x % k` for a name `x` that is never
    /// negative, which Tier 1 proves, for a `match` whose literals cover its refinement.
    fn small_refined(&mut self) -> Option<(S, usize, R)> {
        let (x, _, _) = self.int_name(None, true)?;
        let k = 2 + self.rng.below(3) as u128;
        let name = self.fresh("v");
        let r = R {
            lo: Num::ZERO,
            hi: Num::new(false, k - 1),
        };
        let id = self.declare(name, Ty::Int(U8), Kind::Val, Some(r));
        let init = E::bin(Op::Rem, x, E::Lit(k.to_string()));
        Some((S::Bind(id, Some(format!("u8[0..{}]", k)), init), id, r))
    }

    /// A `match` statement, its arms blocks of statements.
    fn match_statement(&mut self, depth: u32) -> Vec<S> {
        let read = self.read_ty();
        let mut first = Vec::new();
        // In a `Refined` program now and then a refined binding, which literals cover.
        let refined = match (&read, self.mode) {
            (Ty::Int(_), Mode::Refined) if self.rng.chance(40) => self.small_refined(),
            _ => None,
        };
        let scrutinee = match refined {
            Some((bind, id, r)) => {
                first.push(bind);
                Some((self.read(id), Some(r)))
            }
            None => self.scrutinee(&read, depth),
        };
        let Some((s, range)) = scrutinee else {
            return Vec::new();
        };
        let Some(pats) = self.patterns(&read, range, 40) else {
            return Vec::new();
        };
        let mut out = self.flush();
        out.extend(first);
        let root = self.root(&s);
        let mut arms = Vec::new();
        for p in &pats {
            self.scopes.push(Vec::new());
            let ids = self.bind_pattern(p);
            let frozen = self.frozen.len();
            if let Some(r) = root {
                self.frozen.push(r);
            }
            let body = self.block(depth + 1, None, Vec::new(), None);
            self.frozen.truncate(frozen);
            self.scopes.pop();
            arms.push((self.pattern_text(p, &ids), body));
        }
        out.push(S::Match(s, arms));
        out
    }

    fn fields_of(&self, name: &str) -> Vec<GField> {
        self.structs
            .iter()
            .find(|s| s.name == name)
            .map_or_else(Vec::new, |s| s.fields.clone())
    }

    /// A struct built field by field, each value landing in its field's type, a refined
    /// field's inside its refinement.
    fn construct(&mut self, name: &str, depth: u32) -> E {
        let mut fields = Vec::new();
        for f in self.fields_of(name) {
            let v = match (&f.ty, f.refine) {
                (Ty::Int(_), Some(r)) => self.refined_value(r),
                (Ty::Array(n, _), Some(r)) => self.refined_array(*n, r),
                (ty, _) => self.value(ty, depth + 1),
            };
            fields.push((f.name, v));
        }
        E::Construct(name.to_string(), fields)
    }

    /// An array of `n` values Tier 1 proves inside `r`: one repeated, or each its own.
    fn refined_array(&mut self, n: u32, r: R) -> E {
        if n > 0 && n <= 4 && self.rng.chance(50) {
            let m = self.mark();
            let items: Vec<E> = (0..n).map(|_| self.refined_value(r)).collect();
            // Names of integer types no one 128-bit type holds together cannot share a
            // literal, whatever a `guard` says of their values (`E0222`).
            let hull = items
                .iter()
                .filter_map(|e| self.static_range(e))
                .reduce(R::hull);
            if hull.is_none_or(R::fits) {
                return E::Array(items);
            }
            self.restore(m);
        }
        E::Repeat(Box::new(self.refined_value(r)), n)
    }

    /// The range `E0222` takes for `e`, when it is a literal or a name: the literal's value,
    /// and the name's own declared refinement or its type's range (`wide.rs`, `declared`).
    fn static_range(&self, e: &E) -> Option<R> {
        if let Some(k) = constant(e) {
            return Some(R::point(k));
        }
        let E::Name(_) = e else { return None };
        let id = self.root(e)?;
        let Ty::Int(t) = self.binds[id].ty else {
            return None;
        };
        Some(match self.binds[id].kind {
            Kind::Pattern => t.range(),
            Kind::Val | Kind::Var | Kind::Param | Kind::MutParam | Kind::Counter => {
                self.binds[id].range.unwrap_or(t.range())
            }
        })
    }

    /// A value Tier 1 proves inside `r`: a literal, or now and then a name guarded into it.
    fn refined_value(&mut self, r: R) -> E {
        if self.rng.chance(30) {
            if let Some((x, xr, _)) = self.int_name(None, false) {
                if !(r.lo <= xr.lo && xr.hi <= r.hi) {
                    self.pending.push(E::bin(
                        Op::And,
                        E::bin(Op::Ge, x.clone(), E::num(r.lo)),
                        E::bin(Op::Le, x.clone(), E::num(r.hi)),
                    ));
                }
                return x;
            }
        }
        let n = self.value_in(r);
        E::num(n)
    }

    /// A value of struct `name`: a binding's, a call's, a part of another value, or a new one.
    fn struct_value(&mut self, name: &str, depth: u32) -> E {
        let want = Ty::Struct(name.to_string());
        let names: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| self.binds[id].ty == want)
            .collect();
        match self.rng.below(100) {
            0..=29 if !names.is_empty() => {
                let id = *self.rng.pick(&names);
                self.read(id)
            }
            30..=44 => match self.call(depth, |t| *t == want) {
                Some((e, _)) => e,
                None => self.construct(name, depth),
            },
            45..=59 => match self.part(&|t: &Ty| *t == want, true) {
                Some((e, _)) => e,
                None => self.construct(name, depth),
            },
            _ => self.construct(name, depth),
        }
    }

    /// Whether a value of `ty` holds a part `want` accepts, itself or through non-empty arrays
    /// and fields.
    fn reaches(&self, ty: &Ty, want: &dyn Fn(&Ty) -> bool) -> bool {
        want(ty)
            || match ty {
                Ty::Array(n, inner) => *n > 0 && self.reaches(inner, want),
                Ty::Struct(s) => self.fields_of(s).iter().any(|f| self.reaches(&f.ty, want)),
                _ => false,
            }
    }

    /// A part of a visible binding that `want` accepts, down through indexes and fields, with
    /// its type; one below the binding itself when `below`. The binding counts as read.
    fn part(&mut self, want: &dyn Fn(&Ty) -> bool, below: bool) -> Option<(E, Ty)> {
        let ids: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| {
                let ty = &self.binds[id].ty;
                let inner = match ty {
                    Ty::Array(n, inner) => *n > 0 && self.reaches(inner, want),
                    Ty::Struct(s) => self.fields_of(s).iter().any(|f| self.reaches(&f.ty, want)),
                    _ => false,
                };
                inner || (!below && want(ty))
            })
            .collect();
        if ids.is_empty() {
            return None;
        }
        let id = *self.rng.pick(&ids);
        let mut e = self.read(id);
        let mut ty = self.binds[id].ty.clone();
        let mut first = below;
        loop {
            let deeper = match &ty {
                Ty::Array(n, inner) => *n > 0 && self.reaches(inner, want),
                Ty::Struct(s) => self.fields_of(s).iter().any(|f| self.reaches(&f.ty, want)),
                _ => false,
            };
            if want(&ty) && !first && (!deeper || self.rng.chance(50)) {
                return Some((e, ty));
            }
            first = false;
            match ty.clone() {
                Ty::Array(n, inner) if deeper => {
                    // Into an array of structs, now and then by an index that may change it.
                    let call = matches!(*inner, Ty::Struct(_)) && self.rng.chance(60);
                    let i = match call.then(|| self.index_call()).flatten() {
                        Some(c) => E::bin(Op::Rem, c, E::Lit(n.to_string())),
                        None => self.index(n),
                    };
                    e = E::Index(Box::new(e), Box::new(i));
                    ty = *inner;
                }
                Ty::Struct(s) if deeper => {
                    let options: Vec<GField> = self
                        .fields_of(&s)
                        .into_iter()
                        .filter(|f| self.reaches(&f.ty, want))
                        .collect();
                    let f = self.rng.pick(&options).clone();
                    e = E::Field(Box::new(e), f.name);
                    ty = f.ty;
                }
                _ => return want(&ty).then_some((e, ty)),
            }
        }
    }

    /// A value of type `t` that lands anywhere it goes: zero, `false`, or a refinement's least.
    fn default_of(&self, t: &Ty, range: Option<R>) -> E {
        match t {
            Ty::Int(_) => E::num(range.map_or(Num::ZERO, |r| r.lo)),
            Ty::Bool => E::Lit("false".into()),
            // A refinement of an array is its elements'.
            Ty::Array(n, e) => E::Repeat(Box::new(self.default_of(e, range)), *n),
            Ty::Struct(name) => E::Construct(
                name.clone(),
                self.fields_of(name)
                    .into_iter()
                    .map(|f| {
                        let v = self.default_of(&f.ty, f.refine);
                        (f.name, v)
                    })
                    .collect(),
            ),
            // The first variant, its payload at its defaults.
            Ty::Enum(name) => {
                let variants = self.variants_of(name);
                let Some((v, fields)) = variants.first() else {
                    return E::Lit("false".into());
                };
                E::Variant(
                    name.clone(),
                    v.clone(),
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), self.default_of(&f.ty, f.refine)))
                        .collect(),
                )
            }
        }
    }

    // ---------------------------------------------------------------- types and literals

    fn int_ty(&mut self) -> IntT {
        // The 128-bit types are rarer: their operations are checked at every operator.
        *self.rng.pick(&[
            INTS[0], INTS[0], INTS[1], INTS[2], INTS[3], INTS[4], INTS[5], INTS[5], INTS[6],
            INTS[6], INTS[7], INTS[8], INTS[8], INTS[9], INTS[10],
        ])
    }

    fn ty(&mut self, depth: u32) -> Ty {
        if !self.structs.is_empty() && self.rng.chance(15) {
            let k = self.rng.below(self.structs.len() as u64) as usize;
            return Ty::Struct(self.structs[k].name.clone());
        }
        if !self.enums.is_empty() && self.rng.chance(15) {
            let k = self.rng.below(self.enums.len() as u64) as usize;
            return Ty::Enum(self.enums[k].name.clone());
        }
        match self.rng.below(100) {
            0..=69 => Ty::Int(self.int_ty()),
            70..=84 => Ty::Bool,
            _ if depth >= 2 => Ty::Int(self.int_ty()),
            _ => {
                let n = if self.rng.chance(4) {
                    0
                } else {
                    1 + self.rng.below(5) as u32
                };
                let elem = self.ty(depth + 1);
                // Arrays of structs are often of two, as the functions that change them take.
                let n = if matches!(elem, Ty::Struct(_)) && self.rng.chance(60) {
                    2
                } else {
                    n
                };
                Ty::Array(n, Box::new(elem))
            }
        }
    }

    /// A value in `r`, small ones and the extremes most often.
    fn value_in(&mut self, r: R) -> Num {
        let pick = match self.rng.below(10) {
            0..=4 => Num::small(*self.rng.pick(&[0, 1, 2, 3, 4, 5, 7, 8, 10, 16, 31, 100])),
            5 => Num::small(-(self.rng.below(10) as i64)),
            6 => r.lo,
            7 => r.hi,
            _ => {
                let span = r.hi.sub(r.lo).map_or(u128::MAX, |s| s.mag);
                let off = match span.checked_add(1) {
                    Some(m) => self.rng.u128() % m,
                    None => self.rng.u128(),
                };
                r.lo.add(Num::new(false, off)).unwrap_or(r.lo)
            }
        };
        pick.clamp(r.lo, r.hi)
    }

    /// A literal of value `n`, written in one of the forms the lexer reads, with `t`'s suffix.
    fn literal(&mut self, n: Num, suffix: Option<IntT>) -> E {
        let digits = match self.rng.below(20) {
            0 | 1 => format!("0x{:X}", n.mag),
            2 => format!("0b{:b}", n.mag),
            3 if n.mag >= 1000 => {
                let s = n.mag.to_string();
                let mut out = String::new();
                for (k, c) in s.chars().enumerate() {
                    if k > 0 && (s.len() - k).is_multiple_of(3) {
                        out.push('_');
                    }
                    out.push(c);
                }
                out
            }
            _ => n.mag.to_string(),
        };
        let lit = E::Lit(match suffix {
            Some(t) => format!("{}_{}", digits, t.name),
            None => digits,
        });
        if n.neg {
            E::Neg(Box::new(lit))
        } else {
            lit
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{}{}", prefix, self.names)
    }

    fn declare(&mut self, name: String, ty: Ty, kind: Kind, range: Option<R>) -> usize {
        self.binds.push(Binding {
            name,
            ty,
            kind,
            range,
            known: None,
            elements: None,
            read: false,
            assigned: false,
        });
        let id = self.binds.len() - 1;
        self.scopes.last_mut().expect("a scope").push(id);
        id
    }

    /// The bindings an expression may read here.
    fn visible(&self) -> Vec<usize> {
        self.scopes
            .iter()
            .flatten()
            .copied()
            .filter(|id| !self.exclude.contains(id))
            .collect()
    }

    fn read(&mut self, id: usize) -> E {
        self.binds[id].read = true;
        E::name(&self.binds[id].name)
    }

    fn guarded(&self) -> bool {
        self.mode != Mode::Plain
    }

    /// What an attempt changes, so that an attempt given up leaves no trace: a binding it read
    /// but that no statement reads would otherwise go undiscarded (`E0218`).
    fn mark(&self) -> Mark {
        Mark {
            seen: self.binds.iter().map(|b| (b.read, b.assigned)).collect(),
            pending: self.pending.len(),
            cost: self.cost,
            recursed: self.recursed,
        }
    }

    fn restore(&mut self, m: Mark) {
        for (b, (read, assigned)) in self.binds.iter_mut().zip(m.seen) {
            b.read = read;
            b.assigned = assigned;
        }
        self.pending.truncate(m.pending);
        self.cost = m.cost;
        self.recursed = m.recursed;
    }

    // ---------------------------------------------------------------- integer expressions

    fn int_expr(&mut self, want: Want, depth: u32) -> G {
        for _ in 0..6 {
            let m = self.mark();
            if let Some(g) = self.try_int_expr(want, depth) {
                return g;
            }
            self.restore(m);
        }
        self.int_leaf(want)
    }

    fn try_int_expr(&mut self, want: Want, depth: u32) -> Option<G> {
        if depth >= 3 || self.rng.chance(30 + 10 * u64::from(depth)) {
            return Some(self.int_leaf(want));
        }
        match self.rng.below(100) {
            0..=39 => self.arith(want, depth),
            40..=51 => self.bits(want, depth),
            52..=61 => self.shift(want, depth),
            62..=68 => self.negation(want, depth),
            69..=80 => self.int_call(want, depth),
            81..=83 if want == Want::Any => self.edge(false, depth),
            _ => Some(self.int_leaf(want)),
        }
    }

    fn arith(&mut self, want: Want, depth: u32) -> Option<G> {
        let op = *self.rng.pick(&[
            Op::Add,
            Op::Add,
            Op::Sub,
            Op::Sub,
            Op::Mul,
            Op::Div,
            Op::Rem,
        ]);
        let inner = match want {
            Want::Exact(t) => Want::Exact(t),
            Want::Any if self.rng.chance(40) => Want::Exact(self.int_ty()),
            Want::Any => Want::Any,
        };
        let a = self.int_expr(inner, depth + 1);
        let b = match inner {
            Want::Exact(t) if self.rng.chance(30) => {
                let n = self.value_in(t.range());
                G {
                    e: E::num(n),
                    cls: Cls::Lit,
                    r: R::point(n),
                    k: Some(n),
                }
            }
            _ => self.int_expr(inner, depth + 1),
        };
        let b = if matches!(op, Op::Div | Op::Rem) {
            self.divisor(b)
        } else {
            b
        };
        Self::combined(op, a, b)
    }

    /// `a op b` for an arithmetic `op`, with its operands' type, its static range and its
    /// value; none when the range might leave 128 bits, which `E0222` rejects.
    fn combined(op: Op, a: G, b: G) -> Option<G> {
        let cls = a.cls.join(b.cls);
        let wide = a.cls.wide().or(b.cls.wide());
        let r = match wide {
            Some(t) => t.range(),
            // The operands meet the value in one machine type, so they must fit it with it.
            None => arithmetic(op, a.r, b.r).filter(|r| r.hull(a.r).hull(b.r).fits())?,
        };
        let k = match (a.k, b.k) {
            (Some(x), Some(y)) => {
                fold(op, x, y).filter(|v| wide.is_none_or(|t| t.range().holds(*v)))
            }
            _ => None,
        };
        Some(G {
            e: E::bin(op, a.e, b.e),
            cls,
            r,
            k,
        })
    }

    /// A name of an integer binding narrower than 128 bits, or a literal: an operand that
    /// evaluates nothing else.
    fn plain_operand(&mut self) -> G {
        let narrow: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| {
                matches!(self.binds[id].ty, Ty::Int(t) if t.bits < 128)
                    && self.binds[id].kind != Kind::Pattern
            })
            .collect();
        if narrow.is_empty() || self.rng.chance(25) {
            return self.int_literal(Want::Any);
        }
        let id = *self.rng.pick(&narrow);
        let Ty::Int(t) = self.binds[id].ty else {
            return self.int_literal(Want::Any);
        };
        G {
            e: self.read(id),
            cls: Cls::One(t),
            r: self.binds[id].range.unwrap_or(t.range()),
            k: None,
        }
    }

    /// An operand near 2^62, 2^63 or 2^64, on either side of zero: a literal, a small name
    /// added to one or taken from one, or a binding refined there. The product of two lies
    /// around the limit of Tier 1's domain, about 2^125
    /// (`ghost-contracts-and-totality.md`, item 4): at 2^124, half the limit, where the
    /// octagon's constants end; at the limit itself, from either side; at 2^126 and 2^127,
    /// wholly past it; and just below 2^128.
    fn near_edge(&mut self) -> G {
        // A binding whose refinement lies there already.
        let there: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| {
                matches!(self.binds[id].ty, Ty::Int(t) if t.bits < 128)
                    && self.binds[id].range.is_some_and(|r| {
                        r.lo.mag.min(r.hi.mag) >= EDGE_OPERAND && r.lo.neg == r.hi.neg
                    })
            })
            .collect();
        if !there.is_empty() && self.rng.chance(50) {
            let id = *self.rng.pick(&there);
            if let (Ty::Int(t), Some(r)) = (self.binds[id].ty.clone(), self.binds[id].range) {
                return G {
                    e: self.read(id),
                    cls: Cls::One(t),
                    r,
                    k: None,
                };
            }
        }
        let power = 1u128 << *self.rng.pick(&[62u32, 62, 63, 63, 63, 64]);
        let base = match self.rng.below(4) {
            0 => power - 1,
            1 if power < 1 << 64 => power + 1,
            _ if power < 1 << 64 => power,
            _ => power - 1,
        };
        let literal = Num::new(false, base);
        let lit = G {
            e: E::Lit(base.to_string()),
            cls: Cls::Lit,
            r: R::point(literal),
            k: Some(literal),
        };
        let small: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| {
                matches!(self.binds[id].ty, Ty::Int(t) if t.bits <= 32)
                    && self.binds[id].kind != Kind::Pattern
            })
            .collect();
        let g = if !small.is_empty() && self.rng.chance(55) {
            let id = *self.rng.pick(&small);
            let name = match self.binds[id].ty {
                Ty::Int(t) => G {
                    e: E::name(&self.binds[id].name),
                    cls: Cls::One(t),
                    r: self.binds[id].range.unwrap_or(t.range()),
                    k: None,
                },
                _ => return lit,
            };
            let sum = if self.rng.chance(70) {
                Self::combined(Op::Add, name, lit.clone())
            } else {
                Self::combined(Op::Sub, lit.clone(), name)
            };
            // The name is read only where the sum holds it.
            match sum {
                Some(sum) => {
                    self.binds[id].read = true;
                    sum
                }
                None => lit,
            }
        } else {
            lit
        };
        if self.rng.chance(20) {
            let r = R {
                lo: g.r.hi.negated(),
                hi: g.r.lo.negated(),
            };
            if r.hull(g.r).fits() {
                return G {
                    e: E::Neg(Box::new(g.e)),
                    cls: g.cls,
                    r,
                    k: g.k.map(Num::negated),
                };
            }
        }
        g
    }

    /// An expression at the edge of Tier 1's domain: the product of two operands
    /// [`Gen::near_edge`] gives, of which Tier 1 holds a bound weakened to the nearest it has,
    /// then divided, reduced or subtracted, on either side of the operator, by another such
    /// product now and then; and half the time under a dividend, where the value is small
    /// again and an index or a narrow type may take it. `plain` keeps the other operands to
    /// names and literals, as an index needs.
    fn edge(&mut self, plain: bool, depth: u32) -> Option<G> {
        let (a, b) = (self.near_edge(), self.near_edge());
        let product = Self::combined(Op::Mul, a, b)?;
        let op = *self
            .rng
            .pick(&[Op::Div, Op::Div, Op::Sub, Op::Sub, Op::Rem]);
        let other = if self.rng.chance(30) {
            let (c, d) = (self.near_edge(), self.near_edge());
            Self::combined(Op::Mul, c, d)?
        } else if plain {
            self.plain_operand()
        } else {
            self.int_expr(Want::Any, depth + 1)
        };
        let (x, y) = if self.rng.chance(70) {
            (product, other)
        } else {
            (other, product)
        };
        let y = if matches!(op, Op::Div | Op::Rem) {
            self.divisor(y)
        } else {
            y
        };
        let g = Self::combined(op, x, y)?;
        if !self.rng.chance(50) {
            return Some(g);
        }
        let over = if plain {
            self.plain_operand()
        } else {
            self.int_expr(Want::Any, depth + 1)
        };
        let under = self.divisor(g);
        Self::combined(*self.rng.pick(&[Op::Div, Op::Div, Op::Rem]), over, under)
    }

    /// A divisor: one that is always zero becomes one most of the time, and a name is guarded
    /// positive in the modes with guards.
    fn divisor(&mut self, b: G) -> G {
        let zero = b.r == R::point(Num::ZERO) || b.k == Some(Num::ZERO);
        if zero && !self.rng.chance(90) {
            self.zero_divisor = true;
            return b;
        }
        if zero {
            // What names nothing is replaced; what names a binding, as `x - x` and
            // `xs.len - 3` do, keeps its reads and has one added, or a binding only it read
            // would be left unread (`E0218`).
            return if names(&b.e, &|_| true) {
                G {
                    e: E::bin(Op::Add, b.e, E::Lit("1".into())),
                    cls: b.cls,
                    r: R::point(Num::small(1)),
                    k: None,
                }
            } else {
                G {
                    e: E::Lit("1".into()),
                    cls: Cls::Lit,
                    r: R::point(Num::small(1)),
                    k: Some(Num::small(1)),
                }
            };
        }
        if let (E::Name(_), true) = (&b.e, self.guarded() && self.rng.chance(70)) {
            let positive = !b.r.lo.neg || self.rng.chance(60);
            let op = if positive { Op::Gt } else { Op::Lt };
            self.pending
                .push(E::bin(op, b.e.clone(), E::Lit("0".into())));
        }
        b
    }

    fn bits(&mut self, want: Want, depth: u32) -> Option<G> {
        let t = match want {
            Want::Exact(t) => t,
            Want::Any => self.int_ty(),
        };
        let op = *self.rng.pick(&[Op::BitAnd, Op::BitOr, Op::BitXor]);
        let a = self.int_expr(Want::Exact(t), depth + 1);
        let (b, kb) = if self.rng.chance(35) {
            let n = self.value_in(R {
                lo: if t.signed { Num::small(-8) } else { Num::ZERO },
                hi: t.max(),
            });
            (self.literal(n, None), Some(n))
        } else {
            let b = self.int_expr(Want::Exact(t), depth + 1);
            (b.e, b.k)
        };
        let k = match (a.k, kb) {
            (Some(x), Some(y)) if t.range().holds(x) && t.range().holds(y) => {
                let (x, y) = (bits(t, x), bits(t, y));
                Some(unbits(
                    t,
                    match op {
                        Op::BitAnd => x & y,
                        Op::BitOr => x | y,
                        _ => x ^ y,
                    },
                ))
            }
            _ => None,
        };
        Some(G {
            e: E::bin(op, a.e, b),
            cls: Cls::One(t),
            r: t.range(),
            k,
        })
    }

    fn shift(&mut self, want: Want, depth: u32) -> Option<G> {
        let t = match want {
            Want::Exact(t) => t,
            Want::Any => self.int_ty(),
        };
        let op = if self.rng.chance(50) {
            Op::Shl
        } else {
            Op::Shr
        };
        let a = self.int_expr(Want::Exact(t), depth + 1);
        let width = u128::from(t.bits);
        let mut amount = None;
        let k = match self.rng.below(10) {
            0..=5 => {
                let n = if self.rng.chance(5) {
                    width + self.rng.below(2) as u128
                } else {
                    u128::from(self.rng.next()) % width
                };
                amount = Some(n);
                self.literal(Num::new(false, n), None)
            }
            6..=8 => match self.int_name(None, false) {
                Some((e, r, _)) => {
                    if self.guarded() && self.rng.chance(80) {
                        self.bound_guard(&e, r, width - 1);
                        e
                    } else if !r.lo.neg && self.rng.chance(60) {
                        E::bin(Op::Rem, e, E::Lit(width.to_string()))
                    } else {
                        e
                    }
                }
                None => E::Lit("1".into()),
            },
            _ => {
                let k = self.int_expr(Want::Any, depth + 1);
                if k.r.magnitude() > 1 << 64 {
                    return None;
                }
                k.e
            }
        };
        // `x << k` is `x * 2^k` narrowed to `x`'s type; `x >> k` rounds toward negative infinity.
        let value = match (a.k, amount) {
            (Some(x), Some(n)) if n < width && t.range().holds(x) => {
                let p = Num::new(false, 1u128 << n);
                match op {
                    Op::Shl => x.mul(p).filter(|v| t.range().holds(*v)),
                    _ => {
                        let q = Num::new(x.neg, x.mag >> n);
                        Some(if x.neg && x.mag % (1u128 << n) != 0 {
                            q.sub(Num::small(1))?
                        } else {
                            q
                        })
                    }
                }
            }
            _ => None,
        };
        Some(G {
            e: E::bin(op, a.e, k),
            cls: Cls::One(t),
            r: t.range(),
            k: value,
        })
    }

    /// `guard` that `e`, an integer of range `r`, lies in `0 ..= max`.
    fn bound_guard(&mut self, e: &E, r: R, max: u128) {
        let below = E::bin(Op::Le, e.clone(), E::Lit(max.to_string()));
        self.pending.push(if r.lo.neg {
            E::bin(
                Op::And,
                E::bin(Op::Ge, e.clone(), E::Lit("0".into())),
                below,
            )
        } else {
            below
        });
    }

    fn negation(&mut self, want: Want, depth: u32) -> Option<G> {
        let complement = match want {
            Want::Exact(t) => self.rng.chance(40).then_some(t),
            Want::Any => self.rng.chance(20).then(|| self.int_ty()),
        };
        if let Some(t) = complement {
            let a = self.int_expr(Want::Exact(t), depth + 1);
            let k =
                a.k.filter(|x| t.range().holds(*x))
                    .map(|x| unbits(t, !bits(t, x)));
            return Some(G {
                e: E::BitNot(Box::new(a.e)),
                cls: Cls::One(t),
                r: t.range(),
                k,
            });
        }
        let a = self.int_expr(want, depth + 1);
        let r = match a.cls.wide() {
            Some(t) => t.range(),
            None => R {
                lo: a.r.hi.negated(),
                hi: a.r.lo.negated(),
            },
        };
        let literal = matches!(a.e, E::Lit(_));
        if !r.fits() || !(literal || a.cls.wide().is_some() || r.hull(a.r).fits()) {
            return None;
        }
        let wide = a.cls.wide();
        let k =
            a.k.map(Num::negated)
                .filter(|v| wide.is_none_or(|t| t.range().holds(*v)));
        Some(G {
            e: E::Neg(Box::new(a.e)),
            cls: a.cls,
            r,
            k,
        })
    }

    fn int_call(&mut self, want: Want, depth: u32) -> Option<G> {
        let (e, ty) = self.call(depth, |ty| match (ty, want) {
            (Ty::Int(t), Want::Exact(w)) => *t == w,
            (Ty::Int(_), Want::Any) => true,
            _ => false,
        })?;
        let Ty::Int(t) = ty else { return None };
        Some(G {
            e,
            cls: Cls::One(t),
            r: t.range(),
            k: None,
        })
    }

    /// A name of an integer binding, of type `t` when one is given and of a range without
    /// negative values when `non_negative`, with its range and type. It counts as read.
    fn int_name(&mut self, t: Option<IntT>, non_negative: bool) -> Option<(E, R, IntT)> {
        let ids: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| match &self.binds[id].ty {
                Ty::Int(u) => {
                    let r = self.binds[id].range.unwrap_or(u.range());
                    t.is_none_or(|t| t == *u) && !(non_negative && r.lo.neg)
                }
                _ => false,
            })
            .collect();
        if ids.is_empty() {
            return None;
        }
        let id = *self.rng.pick(&ids);
        let Ty::Int(u) = self.binds[id].ty else {
            return None;
        };
        let r = self.binds[id].range.unwrap_or(u.range());
        Some((self.read(id), r, u))
    }

    fn int_leaf(&mut self, want: Want) -> G {
        let exact = match want {
            Want::Exact(t) => Some(t),
            Want::Any => None,
        };
        let roll = self.rng.below(100);
        if roll < 50 {
            if let Some((e, r, t)) = self.int_name(exact, false) {
                // The range `E0222` takes for a name is its own declared refinement's, which a
                // pattern's binding has none of: its field's refinement is a fact for Tier 1
                // alone (`wide.rs`, `declared`).
                let pattern = self
                    .root(&e)
                    .is_some_and(|id| self.binds[id].kind == Kind::Pattern);
                return G {
                    e,
                    cls: Cls::One(t),
                    r: if pattern { t.range() } else { r },
                    k: None,
                };
            }
        } else if roll < 75 {
            if let Some(g) = self.element(want) {
                return g;
            }
        } else if roll < 80 && exact.is_none_or(|t| t == USIZE) {
            let arrays: Vec<(usize, u32)> = self
                .visible()
                .into_iter()
                .filter_map(|id| match self.binds[id].ty {
                    Ty::Array(n, _) => Some((id, n)),
                    _ => None,
                })
                .collect();
            if !arrays.is_empty() {
                let (id, n) = *self.rng.pick(&arrays);
                let base = self.read(id);
                let len = Num::new(false, u128::from(n));
                return G {
                    e: E::Len(Box::new(base)),
                    cls: Cls::One(USIZE),
                    r: R::point(len),
                    k: Some(len),
                };
            }
        }
        self.int_literal(want)
    }

    fn int_literal(&mut self, want: Want) -> G {
        match want {
            Want::Exact(t) => {
                let n = self.value_in(R {
                    lo: Num::ZERO,
                    hi: t.max(),
                });
                G {
                    e: self.literal(n, Some(t)),
                    cls: Cls::One(t),
                    r: R::point(n),
                    k: Some(n),
                }
            }
            Want::Any => {
                let t = self.int_ty();
                let n = self.value_in(t.range());
                G {
                    e: self.literal(n, None),
                    cls: Cls::Lit,
                    r: R::point(n),
                    k: Some(n),
                }
            }
        }
    }

    /// An element of an array binding or a field of a struct, down through both, to an
    /// integer of the type `want` asks for. Its range is its type's, which `E0222` takes for a
    /// field, refined or not.
    fn element(&mut self, want: Want) -> Option<G> {
        let accept = |t: &Ty| match t {
            Ty::Int(u) => !matches!(want, Want::Exact(w) if w != *u),
            _ => false,
        };
        let (e, ty) = self.part(&accept, true)?;
        let Ty::Int(t) = ty else { return None };
        Some(G {
            e,
            cls: Cls::One(t),
            r: t.range(),
            k: None,
        })
    }

    /// An index into an array of length `n`: in range most of the time, by a literal, a loop's
    /// counter, a remainder or a guarded name.
    fn index(&mut self, n: u32) -> E {
        let n = u128::from(n);
        let safe: Vec<E> = self
            .safe
            .iter()
            .filter(|(_, max)| *max < n)
            .map(|(e, _)| e.clone())
            .collect();
        match self.rng.below(100) {
            _ if n == 0 => E::Lit("0".into()),
            0..=34 if !safe.is_empty() => self.rng.pick(&safe).clone(),
            0..=44 => {
                let k = u128::from(self.rng.next()) % n;
                E::Lit(k.to_string())
            }
            45..=79 => match self.int_name(None, false) {
                Some((e, r, _)) => {
                    if self.guarded() && self.rng.chance(85) {
                        self.bound_guard(&e, r, n - 1);
                    }
                    if !self.guarded() && !r.lo.neg && self.rng.chance(40) {
                        E::bin(Op::Rem, e, E::Lit(n.to_string()))
                    } else {
                        e
                    }
                }
                None => E::Lit("0".into()),
            },
            80..=89 => match self.int_name(None, true) {
                Some((e, ..)) => E::bin(Op::Rem, e, E::Lit(n.to_string())),
                None => E::Lit((n - 1).to_string()),
            },
            90..=95 => match self.index_call() {
                Some(e) => E::bin(Op::Rem, e, E::Lit(n.to_string())),
                None => E::Lit((n - 1).to_string()),
            },
            // A value computed at the edge of Tier 1's domain that may lie within the array.
            96..=97 => {
                let within = R {
                    lo: Num::ZERO,
                    hi: Num::new(false, n - 1),
                };
                for _ in 0..4 {
                    let m = self.mark();
                    match self.edge(true, 3) {
                        Some(g) if g.r.meets(within) && g.k.is_none_or(|v| within.holds(v)) => {
                            return g.e;
                        }
                        _ => self.restore(m),
                    }
                }
                E::Lit((n - 1).to_string())
            }
            _ => E::Lit(n.to_string()),
        }
    }

    /// A call whose result is an integer, for an index, which traps where it is negative: one
    /// with `mut` arguments may change the array it indexes, which is read as it was before the index is evaluated
    /// (`core-calculus.md`, items 10 and 12). Its own arguments make no further calls.
    fn index_call(&mut self) -> Option<E> {
        if self.no_calls {
            return None;
        }
        let options: Vec<Sig> = self
            .sigs
            .iter()
            .filter(|s| matches!(s.ret, Some(Ty::Int(_))))
            .filter(|s| s.params.iter().any(|p| p.is_mut))
            .filter(|s| self.cost + self.mult * s.cost <= COST_BUDGET)
            .cloned()
            .collect();
        if options.is_empty() {
            return None;
        }
        let sig = self.rng.pick(&options).clone();
        let m = self.mark();
        self.no_calls = true;
        let e = self.arguments(&sig, false, 2);
        self.no_calls = false;
        match e {
            Some(e) => {
                self.cost += self.mult * sig.cost;
                Some(e)
            }
            None => {
                self.restore(m);
                None
            }
        }
    }

    // ---------------------------------------------------------------- other expressions

    fn bool_expr(&mut self, depth: u32) -> E {
        if depth >= 3 || self.rng.chance(25 + 15 * u64::from(depth)) {
            return self.bool_leaf();
        }
        match self.rng.below(100) {
            0..=44 => {
                let op = *self
                    .rng
                    .pick(&[Op::Eq, Op::Ne, Op::Lt, Op::Le, Op::Gt, Op::Ge]);
                let a = self.int_expr(Want::Any, depth + 1);
                let b = if self.rng.chance(50) {
                    let n = self.value_in(a.r);
                    self.literal(n, None)
                } else {
                    self.int_expr(Want::Any, depth + 1).e
                };
                E::bin(op, a.e, b)
            }
            45..=59 => {
                let op = if self.rng.chance(50) { Op::And } else { Op::Or };
                let a = self.bool_expr(depth + 1);
                let b = self.bool_expr(depth + 1);
                E::bin(op, a, b)
            }
            60..=69 => E::Not(Box::new(self.bool_expr(depth + 1))),
            70..=74 => {
                let op = if self.rng.chance(50) { Op::Eq } else { Op::Ne };
                let a = self.bool_expr(depth + 1);
                let b = self.bool_expr(depth + 1);
                E::bin(op, a, b)
            }
            75..=86 => match self.call(depth, |t| *t == Ty::Bool) {
                Some((e, _)) => e,
                None => self.bool_leaf(),
            },
            _ => self.bool_leaf(),
        }
    }

    fn bool_leaf(&mut self) -> E {
        if self.rng.chance(75) {
            if let Some((e, _)) = self.part(&|t: &Ty| *t == Ty::Bool, false) {
                return e;
            }
        }
        E::Lit(if self.rng.chance(50) { "true" } else { "false" }.into())
    }

    /// A value of type `ty`, for a place of that type.
    fn value(&mut self, ty: &Ty, depth: u32) -> E {
        match ty {
            Ty::Int(t) => self.narrowed(*t, depth),
            Ty::Bool => self.bool_expr(depth),
            Ty::Array(n, elem) => self.array(*n, elem, depth),
            Ty::Struct(name) => self.struct_value(name, depth),
            Ty::Enum(name) => self.enum_value(name, depth),
        }
    }

    /// What a run can know of the integer binding `id`, of type `t`, where generation stands:
    /// what was followed of it, within its refinement or its type.
    fn known_range(&self, id: usize, t: IntT) -> R {
        let base = self.binds[id].range.unwrap_or(t.range());
        self.binds[id]
            .known
            .and_then(|k| k.meet(base))
            .unwrap_or(base)
    }

    /// What a run can know of the integer `e` gives, as Tier 1 follows it: a literal's value, a
    /// name's range where it stands, the sums, differences, products and negations of those,
    /// one shifted left by a literal, and a quotient of two that are never negative. None for
    /// anything else, of which the generator knows its static range alone.
    fn fact(&self, e: &E) -> Option<R> {
        match e {
            E::Lit(text) => literal_value(text).map(R::point),
            E::Name(_) => {
                let id = self.root(e)?;
                match self.binds[id].ty {
                    Ty::Int(t) => Some(self.known_range(id, t)),
                    _ => None,
                }
            }
            E::Neg(x) => {
                let r = self.fact(x)?;
                Some(R {
                    lo: r.hi.negated(),
                    hi: r.lo.negated(),
                })
            }
            E::Bin(op @ (Op::Add | Op::Sub | Op::Mul), a, b) => {
                arithmetic(*op, self.fact(a)?, self.fact(b)?)
            }
            // `x << k` is `x * 2^k`, where it does not trap.
            E::Bin(Op::Shl, a, k) => {
                let k = u32::try_from(constant(k).filter(|k| !k.neg)?.mag).ok()?;
                let power = Num::new(false, 1u128.checked_shl(k)?);
                arithmetic(Op::Mul, self.fact(a)?, R::point(power))
            }
            // A quotient of values that are never negative, its divisor never zero where the
            // division runs: between the least over the greatest and the greatest over the
            // least.
            E::Bin(Op::Div, a, b) => {
                let (a, b) = (self.fact(a)?, self.fact(b)?);
                if a.lo.neg || b.lo.neg || b.hi.mag == 0 {
                    return None;
                }
                Some(R {
                    lo: Num::new(false, a.lo.mag / b.hi.mag),
                    hi: Num::new(false, a.hi.mag / b.lo.mag.max(1)),
                })
            }
            _ => None,
        }
    }

    /// Whether `e` may land in `t`: nothing a run can know of it places it wholly outside the
    /// type, where its narrowing would always trap and Tier 1 refuse it (`E0508`). A value
    /// that may trap or may not is what the comparison of the paths is for, and stays.
    fn may_fit(&self, e: &E, t: IntT) -> bool {
        self.fact(e).is_none_or(|f| f.meets(t.range()))
    }

    /// What binding `id` holds once `init` initializes it: what a run can know of `init`,
    /// within the binding's type.
    fn initialized(&mut self, id: usize, init: &E) {
        if let Ty::Int(t) = self.binds[id].ty {
            self.binds[id].known = self.fact(init).and_then(|f| f.meet(t.range()));
        }
    }

    /// Takes what a `guard` on `c` leaves of the names it bounds by a literal: `x < k`,
    /// `k <= x` and the like, and those joined by `and`.
    fn learn(&mut self, c: &E) {
        let E::Bin(op, a, b) = c else { return };
        if *op == Op::And {
            self.learn(a);
            self.learn(b);
            return;
        }
        // `x op k`, or `k op x` turned round.
        let (x, op, k) = match (constant(a), constant(b)) {
            (None, Some(k)) => (a, *op, k),
            (Some(k), None) => {
                let turned = match op {
                    Op::Lt => Op::Gt,
                    Op::Le => Op::Ge,
                    Op::Gt => Op::Lt,
                    Op::Ge => Op::Le,
                    other => *other,
                };
                (b, turned, k)
            }
            _ => return,
        };
        let E::Name(_) = &**x else { return };
        let Some(id) = self.root(x) else { return };
        let Ty::Int(t) = self.binds[id].ty else {
            return;
        };
        let now = self.known_range(id, t);
        let one = Num::small(1);
        let bound = match op {
            Op::Lt => k.sub(one).map(|hi| R { lo: now.lo, hi }),
            Op::Le => Some(R { lo: now.lo, hi: k }),
            Op::Gt => k.add(one).map(|lo| R { lo, hi: now.hi }),
            Op::Ge => Some(R { lo: k, hi: now.hi }),
            Op::Eq => Some(R::point(k)),
            _ => None,
        };
        // A `guard` no value passes leaves nothing to know: what follows it never runs.
        if let Some(r) = bound.and_then(|b| now.meet(b)) {
            self.binds[id].known = Some(r);
        }
    }

    /// An integer expression whose static range meets `t`'s, since one wholly outside it would
    /// always trap where it lands (`E0508`).
    fn narrowed(&mut self, t: IntT, depth: u32) -> E {
        self.narrowed_range(t, depth).0
    }

    /// [`Gen::narrowed`], with the expression's static range.
    fn narrowed_range(&mut self, t: IntT, depth: u32) -> (E, R) {
        // Now and then a value computed at the edge of Tier 1's domain.
        let edge = self.rng.chance(6);
        for _ in 0..6 {
            let mut m = self.mark();
            let tried = if edge { self.edge(false, depth) } else { None };
            let g = match tried {
                Some(g) => g,
                None => {
                    // What the attempt read, it no longer reads.
                    self.restore(m);
                    m = self.mark();
                    self.int_expr(Want::Any, depth)
                }
            };
            if g.r.meets(t.range())
                && g.k.is_none_or(|v| t.range().holds(v))
                && self.may_fit(&g.e, t)
            {
                return (g.e, g.r);
            }
            self.restore(m);
        }
        let n = self.value_in(t.range());
        (self.literal(n, None), R::point(n))
    }

    fn array(&mut self, n: u32, elem: &Ty, depth: u32) -> E {
        // What this value's own parts hold, such as a call's arguments, is nothing to the array
        // literal around it; the value itself is, with the range of the integers it gives.
        let (e, hull) = self.apart(|g| g.array_value(n, elem, depth));
        if let (Some(top), Some(r)) = (self.hulls.last_mut(), hull) {
            *top = Some(top.map_or(r, |h| h.hull(r)));
        }
        e
    }

    /// What `f` generates, apart from the array literal being generated around it.
    fn apart<T>(&mut self, f: impl FnOnce(&mut Gen) -> T) -> T {
        self.hulls.push(None);
        let v = f(self);
        self.hulls.pop();
        v
    }

    /// A value of `[n]elem`, with the range of the integers it gives an array literal that
    /// holds it: a literal's own hull, and for any other array the range of its own element
    /// type, which may be another integer type than `elem` (`wide.rs`, `leaves`).
    fn array_value(&mut self, n: u32, elem: &Ty, depth: u32) -> (E, Option<R>) {
        let want = Ty::Array(n, Box::new(elem.clone()));
        let names: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| flows(&self.binds[id].ty, &want))
            .collect();
        match self.rng.below(100) {
            0..=39 if !names.is_empty() => {
                let id = *self.rng.pick(&names);
                let typed = leaf_range(&self.binds[id].ty);
                (self.read(id), typed)
            }
            40..=49 => match self.call(depth, |t| flows(t, &want)) {
                Some((e, ty)) => (e, leaf_range(&ty)),
                None => self.array_literal(n, elem, depth),
            },
            50..=59 => {
                // An element of a nested array.
                let outer: Vec<(usize, u32)> = self
                    .visible()
                    .into_iter()
                    .filter_map(|id| match &self.binds[id].ty {
                        Ty::Array(m, inner) if flows(inner, &want) => Some((id, *m)),
                        _ => None,
                    })
                    .collect();
                match outer.as_slice() {
                    [] => self.array_literal(n, elem, depth),
                    _ => {
                        let (id, m) = *self.rng.pick(&outer);
                        let typed = leaf_range(&self.binds[id].ty);
                        let base = self.read(id);
                        let i = self.index(m);
                        (E::Index(Box::new(base), Box::new(i)), typed)
                    }
                }
            }
            _ => self.array_literal(n, elem, depth),
        }
    }

    /// An array literal of `n` values of type `elem`, with the hull of the ranges of the
    /// integers it holds, through the literals nested in it; none for an array of no integers.
    /// Integers of different types are kept exact in one machine type until the array lands,
    /// those of nested literals together, so their hull must lie within `u128` or within `i128`
    /// (`E0222`); elements whose hull does not are generated again, and then a repeat of one
    /// value stands in.
    fn array_literal(&mut self, n: u32, elem: &Ty, depth: u32) -> (E, Option<R>) {
        let repeat = n == 0 || self.rng.chance(30);
        match elem {
            Ty::Int(t) if !repeat => {
                for _ in 0..4 {
                    let m = self.mark();
                    let items: Vec<(E, R)> =
                        self.apart(|g| (0..n).map(|_| g.narrowed_range(*t, depth + 1)).collect());
                    let hull = items
                        .iter()
                        .skip(1)
                        .fold(items[0].1, |h, (_, r)| h.hull(*r));
                    if hull.fits() {
                        let items = items.into_iter().map(|(e, _)| e).collect();
                        return (E::Array(items), Some(hull));
                    }
                    self.restore(m);
                }
            }
            Ty::Array(..) if !repeat => {
                for _ in 0..4 {
                    let m = self.mark();
                    let (items, hull) = self.elements(n, elem, depth);
                    if hull.is_none_or(|h| h.fits()) {
                        return (E::Array(items), hull);
                    }
                    self.restore(m);
                }
            }
            Ty::Struct(_) | Ty::Enum(_) | Ty::Bool if !repeat => {
                let (items, _) = self.elements(n, elem, depth);
                return (E::Array(items), None);
            }
            Ty::Int(_) | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) | Ty::Bool => {}
        }
        // One value repeated: its integers fit a machine type by themselves.
        match elem {
            Ty::Int(t) => {
                let (v, r) = self.apart(|g| g.narrowed_range(*t, depth + 1));
                (E::Repeat(Box::new(v), n), Some(r))
            }
            Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) | Ty::Bool => {
                let (mut items, hull) = self.elements(1, elem, depth);
                (E::Repeat(Box::new(items.remove(0)), n), hull)
            }
        }
    }

    /// `n` values of type `elem`, with the hull of the ranges of the integers the arrays among
    /// them give.
    fn elements(&mut self, n: u32, elem: &Ty, depth: u32) -> (Vec<E>, Option<R>) {
        self.hulls.push(None);
        let items = (0..n).map(|_| self.value(elem, depth + 1)).collect();
        let hull = self.hulls.pop().flatten();
        (items, leaf_range(elem).and(hull))
    }

    /// A value of type `ty` in a place a full expression stands: sometimes an `if`.
    ///
    /// An integer `if` takes the hull of its branches' ranges, which must lie within `u128` or
    /// within `i128` (`E0222`), so branches whose hull does not are generated again.
    fn full(&mut self, ty: &Ty, depth: u32) -> E {
        // Now and then a `match`, its arms meeting as an `if`'s branches do.
        if !matches!(ty, Ty::Array(..)) && depth < 3 && self.rng.chance(12) {
            let m = self.mark();
            if let Some((e, _)) = self.match_value(ty, depth) {
                return e;
            }
            self.restore(m);
        }
        if !matches!(ty, Ty::Array(..)) && self.rng.chance(10) {
            let m = self.mark();
            let c = self.bool_expr(depth + 1);
            let (a, b, hull) = match ty {
                Ty::Int(t) => {
                    let (a, ra) = self.narrowed_range(*t, depth + 1);
                    let (b, rb) = self.narrowed_range(*t, depth + 1);
                    (a, b, ra.hull(rb))
                }
                _ => (
                    self.value(ty, depth + 1),
                    self.value(ty, depth + 1),
                    R::point(Num::ZERO),
                ),
            };
            if hull.fits() {
                return E::If(Box::new(c), Box::new(a), Box::new(b));
            }
            self.restore(m);
        }
        self.value(ty, depth)
    }

    /// A call of a function defined before, or of this one when it recurses, whose result type
    /// `fits`, within the cost budget; its arguments narrow into its parameters, a `mut` one is a
    /// place of its type exactly, and what its contracts need is guarded before the statement.
    fn call(&mut self, depth: u32, fits: impl Fn(&Ty) -> bool) -> Option<(E, Ty)> {
        self.call_of(depth, fits, false)
    }

    /// [`Gen::call`], of a function with a `mut` parameter alone when `changes`.
    fn call_of(
        &mut self,
        depth: u32,
        fits: impl Fn(&Ty) -> bool,
        changes: bool,
    ) -> Option<(E, Ty)> {
        if self.no_calls || depth >= 3 {
            return None;
        }
        let wanted = |s: &Sig| {
            s.ret.as_ref().is_some_and(&fits) && (!changes || s.params.iter().any(|p| p.is_mut))
        };
        let mut options: Vec<Sig> = self
            .sigs
            .iter()
            .filter(|s| wanted(s))
            .filter(|s| self.cost + self.mult * s.cost <= COST_BUDGET)
            .cloned()
            .collect();
        let recursive = match (&self.current, self.recursion) {
            (Some(s), Some(_)) if !self.recursed && self.loops == 0 => wanted(s).then(|| s.clone()),
            _ => None,
        };
        if let Some(s) = &recursive {
            if self.rng.chance(50) {
                options.clear();
            }
            options.push(s.clone());
        }
        if options.is_empty() {
            return None;
        }
        // A function that stays small calls, more often than not, a function that relates two
        // of its parameters when there is one: only there may the arguments be two names.
        let relating: Vec<Sig> = options
            .iter()
            .filter(|s| s.related().iter().any(|r| !r.is_empty()))
            .cloned()
            .collect();
        if self.relates() && !relating.is_empty() && self.rng.chance(60) {
            options = relating;
        }
        let sig = self.rng.pick(&options).clone();
        let is_self = recursive.as_ref().is_some_and(|s| s.name == sig.name);
        let m = self.mark();
        let Some(e) = self.arguments(&sig, is_self, depth) else {
            self.restore(m);
            return None;
        };
        if is_self {
            self.recursed = true;
        } else {
            self.cost += self.mult * sig.cost;
        }
        Some((e, sig.ret.clone()?))
    }

    /// A call of `sig` with its arguments, or `None` when no place fits a `mut` parameter.
    fn arguments(&mut self, sig: &Sig, is_self: bool, depth: u32) -> Option<E> {
        let saved = self.exclude.len();
        let mut places: Vec<Option<E>> = vec![None; sig.params.len()];
        for (k, p) in sig.params.iter().enumerate() {
            if p.is_mut {
                let Some((id, place)) = self.place(Some(&p.ty)) else {
                    self.exclude.truncate(saved);
                    return None;
                };
                self.exclude.push(id);
                places[k] = Some(place);
            }
        }
        for &id in &self.exclude[saved..] {
            self.binds[id].assigned = true;
            self.binds[id].known = None;
        }
        let mut args: Vec<(bool, E)> = Vec::new();
        // Arguments a refinement bound or a `requires` names are names or literals, so that a
        // guard can state what the call needs of them.
        let mut simple: Vec<bool> = vec![false; sig.params.len()];
        for r in &sig.requires {
            match *r {
                Req::Bound(a, ..) => simple[a] = true,
                Req::Order(a, _, b) | Req::Sum(a, b, _) | Req::Below(a, b) => {
                    simple[a] = true;
                    simple[b] = true;
                }
            }
        }
        for p in &sig.params {
            if let Some(r) = &p.refine {
                for b in [&r.lo, &r.hi] {
                    if let Bnd::Name(i) | Bnd::Len(i) = b {
                        simple[*i] = true;
                    }
                }
            }
        }
        // The parameters a refinement bound or a `requires` relates to each one: its
        // arguments are two values the call must be proven to relate.
        let related = sig.related();
        let relates = self.relates();
        // The most an argument may be that a `requires` adds to another: a literal above it
        // never meets the clause, and one near the limit of a 128-bit type makes the sum the
        // guard states leave 128 bits (`E0222`).
        let mut most: Vec<Option<Num>> = vec![None; sig.params.len()];
        for r in &sig.requires {
            if let Req::Sum(a, b, c) = *r {
                for k in [a, b] {
                    most[k] = Some(most[k].map_or(c, |m| m.min(c)));
                }
            }
        }
        for (k, p) in sig.params.iter().enumerate() {
            if let Some(place) = places[k].take() {
                args.push((true, place));
                continue;
            }
            let e = if is_self && k == 0 {
                E::bin(Op::Sub, E::name(&p.name), E::Lit("1".into()))
            } else if p.refine.is_some() || simple[k] {
                let literal = p.refine.as_ref().and_then(|r| match (&r.lo, &r.hi) {
                    (Bnd::Lit(lo), Bnd::Lit(hi)) if r.inclusive => Some(R { lo: *lo, hi: *hi }),
                    (Bnd::Lit(lo), Bnd::Lit(hi)) => Some(R {
                        lo: *lo,
                        hi: hi.sub(Num::small(1))?,
                    }),
                    _ => None,
                });
                let literal = literal.or(match (&p.ty, most[k]) {
                    (Ty::Int(t), Some(c)) => t.range().meet(R {
                        lo: Num::ZERO,
                        hi: c,
                    }),
                    _ => None,
                });
                // The arguments before this one that it is related to, which name a binding.
                let named: Vec<&E> = related[k]
                    .iter()
                    .filter(|&&j| j < k)
                    .map(|&j| &args[j].1)
                    .filter(|a| names(a, &|_| true))
                    .collect();
                // Tier 1 relates two names only in a function that stays within the
                // variables it relates ([`Gen::relates`]); past them it holds each one's
                // bounds alone, which prove a name against a literal and nothing of two
                // names. And it holds no sum of a name with itself, `n + n`, in any function.
                // So an argument related to a name is a literal in a function that may grow,
                // and is another name than that one in a function that stays small.
                let m = self.mark();
                // Where two names may be related, a related argument is a name whenever one
                // is in scope.
                let name = match (&p.ty, relates && !related[k].is_empty()) {
                    (Ty::Int(_), true) => self.int_name(None, false).map(|(e, ..)| e),
                    _ => None,
                };
                let e = match name {
                    Some(e) => e,
                    None => self.simple(&p.ty, literal),
                };
                let again = named
                    .iter()
                    .any(|a| !relates || names(a, &|n| names(&e, &|x| x == n)));
                match (&p.ty, again) {
                    (Ty::Int(t), true) => {
                        // The name is not read after all. A literal near the least value
                        // the parameter takes meets what relates it to a name most often.
                        self.restore(m);
                        let all = literal.unwrap_or(t.range());
                        let least = match p.refine.as_ref().map(|r| &r.lo) {
                            Some(Bnd::Lit(lo)) => *lo,
                            Some(Bnd::Name(_) | Bnd::Len(_)) | None => Num::ZERO,
                        };
                        let few = least
                            .add(Num::small(3))
                            .and_then(|hi| all.meet(R { lo: least, hi }));
                        let n = self.value_in(few.unwrap_or(all));
                        self.literal(n, None)
                    }
                    _ => e,
                }
            } else {
                self.value(&p.ty, depth + 1)
            };
            args.push((false, e));
        }
        self.exclude.truncate(saved);
        // What the callee's refinements and `requires` need, stated over the arguments.
        let arg = |k: usize| args[k].1.clone();
        for (k, p) in sig.params.iter().enumerate() {
            let Some(r) = &p.refine else { continue };
            if is_self && k == 0 {
                continue;
            }
            let bound = |b: &Bnd| match b {
                Bnd::Lit(n) => E::num(*n),
                Bnd::Name(i) => arg(*i),
                Bnd::Len(i) => match &sig.params[*i].ty {
                    Ty::Array(n, _) => E::Lit(n.to_string()),
                    _ => E::Lit("0".into()),
                },
            };
            let lo = E::bin(Op::Ge, arg(k), bound(&r.lo));
            let hi = E::bin(
                if r.inclusive { Op::Le } else { Op::Lt },
                arg(k),
                bound(&r.hi),
            );
            self.pending.push(E::bin(Op::And, lo, hi));
        }
        for r in &sig.requires {
            let len = |i: usize| match &sig.params[i].ty {
                Ty::Array(n, _) => E::Lit(n.to_string()),
                _ => E::Lit("0".into()),
            };
            self.pending.push(match *r {
                Req::Bound(a, op, c) => E::bin(op, arg(a), E::num(c)),
                Req::Order(a, op, b) => E::bin(op, arg(a), arg(b)),
                Req::Sum(a, b, c) => E::bin(Op::Le, E::bin(Op::Add, arg(a), arg(b)), E::num(c)),
                Req::Below(a, b) => E::bin(Op::Lt, arg(a), len(b)),
            });
        }
        Some(E::Call(sig.name.clone(), args))
    }

    /// An argument a guard can speak of: a name of a binding of an integer type, or a literal,
    /// in `literal` when it is given; anything else for other types.
    fn simple(&mut self, ty: &Ty, literal: Option<R>) -> E {
        match ty {
            Ty::Int(t) => match self.rng.chance(75).then(|| self.int_name(None, false)) {
                Some(Some((e, ..))) => e,
                _ => {
                    let n = self.value_in(literal.unwrap_or(t.range()));
                    E::num(n)
                }
            },
            _ => self.value(ty, 2),
        }
    }

    /// A place of type `ty` exactly, or of any type: a `var` or a `mut` parameter, or an element
    /// or a field of one, down to four steps. A place for a `mut` argument, of a type given, is
    /// never a refined field, whose refinement the callee's value would have to be proven in on
    /// its way back. Returns its root binding.
    fn place(&mut self, ty: Option<&Ty>) -> Option<(usize, E)> {
        let mut options: Vec<(usize, Vec<PStep>)> = Vec::new();
        for id in self.visible() {
            // A place a `match` arm reads, through what its pattern binds, stays as it is.
            if !matches!(self.binds[id].kind, Kind::Var | Kind::MutParam)
                || self.frozen.contains(&id)
            {
                continue;
            }
            // Each step into the binding gives a place.
            let mut todo: Vec<(Ty, Vec<PStep>, bool)> =
                vec![(self.binds[id].ty.clone(), Vec::new(), false)];
            while let Some((t, steps, refined)) = todo.pop() {
                if ty.is_none_or(|w| *w == t && !refined) {
                    options.push((id, steps.clone()));
                }
                if steps.len() >= 4 {
                    continue;
                }
                match &t {
                    Ty::Array(n, inner) if *n > 0 => {
                        let mut s = steps.clone();
                        s.push(PStep::Index(*n));
                        todo.push(((**inner).clone(), s, refined));
                    }
                    Ty::Struct(name) => {
                        for f in self.fields_of(name) {
                            let mut s = steps.clone();
                            s.push(PStep::Field(f.name.clone()));
                            todo.push((f.ty, s, f.refine.is_some()));
                        }
                    }
                    _ => {}
                }
            }
        }
        if options.is_empty() {
            return None;
        }
        let (id, steps) = self.rng.pick(&options).clone();
        let mut e = E::name(&self.binds[id].name);
        self.exclude.push(id);
        for step in steps {
            e = match step {
                PStep::Index(n) => {
                    let i = self.index(n);
                    E::Index(Box::new(e), Box::new(i))
                }
                PStep::Field(f) => E::Field(Box::new(e), f),
            };
        }
        self.exclude.pop();
        Some((id, e))
    }

    fn place_ty(&self, id: usize, e: &E) -> Ty {
        match e {
            E::Index(b, _) => match self.place_ty(id, b) {
                Ty::Array(_, inner) => *inner,
                t => t,
            },
            E::Field(b, f) => match self.place_ty(id, b) {
                Ty::Struct(s) => self
                    .fields_of(&s)
                    .into_iter()
                    .find(|g| g.name == *f)
                    .map_or(Ty::Bool, |g| g.ty),
                t => t,
            },
            _ => self.binds[id].ty.clone(),
        }
    }

    /// The refinement of a field place, when it is one: of an integer field, or of an array
    /// field's elements, which an element of it has too.
    fn place_refine(&self, id: usize, e: &E) -> Option<R> {
        match e {
            E::Field(b, f) => {
                let Ty::Struct(s) = self.place_ty(id, b) else {
                    return None;
                };
                self.fields_of(&s)
                    .into_iter()
                    .find(|g| g.name == *f)
                    .and_then(|g| g.refine)
            }
            E::Index(b, _) => match (self.place_ty(id, b), &**b) {
                // An element of what a pattern bound to an array field, or of the field itself.
                (Ty::Array(_, inner), E::Name(_)) if matches!(*inner, Ty::Int(_)) => {
                    self.binds[id].elements
                }
                (Ty::Array(_, inner), _) if matches!(*inner, Ty::Int(_)) => {
                    self.place_refine(id, b)
                }
                _ => None,
            },
            _ => None,
        }
    }

    // ---------------------------------------------------------------- statements

    fn flush(&mut self) -> Vec<S> {
        let conds = std::mem::take(&mut self.pending);
        conds
            .into_iter()
            .map(|c| {
                self.learn(&c);
                let leave = self.leave();
                S::Guard(
                    c,
                    Block {
                        stmts: vec![leave],
                        tail: None,
                    },
                )
            })
            .collect()
    }

    /// A statement that leaves: the loop's next round or its end inside a loop, a return, or
    /// now and then a `todo` or an `unreachable`.
    fn leave(&mut self) -> S {
        let roll = self.rng.below(100);
        if self.loops > 0 && roll < 30 {
            return S::Continue;
        }
        if self.loops > 0 && roll < 50 {
            return S::Break;
        }
        match roll {
            95..=96 => S::Expr(E::Builtin("todo", "not handled".into())),
            97 => S::Expr(E::Builtin("unreachable", "ruled out".into())),
            _ => S::Return(self.current.as_ref().and_then(|s| s.ret.clone()).map(|t| {
                let r = self.current.as_ref().and_then(|s| s.ret_range);
                self.default_of(&t, r)
            })),
        }
    }

    /// A block of statements, the first ones given, ending with a value of type `value` when
    /// one is asked for; `clamp`, a function's contract on its result, binds the value first and
    /// returns a literal inside the contract unless a `guard` finds the value inside it too.
    fn block(&mut self, depth: u32, value: Option<&Ty>, first: Vec<S>, clamp: Option<R>) -> Block {
        let before: Vec<Option<R>> = self.binds.iter().map(|b| b.known).collect();
        self.scopes.push(Vec::new());
        let mut stmts = first;
        let count = 1 + self.rng.below(if depth == 0 { 6 } else { 4 });
        let mut ended = false;
        for _ in 0..count {
            if self.cost > COST_BUDGET || (self.relates() && self.binds.len() >= RELATED) {
                break;
            }
            let s = self.statement(depth);
            ended = s.last().is_some_and(S::leaves);
            stmts.extend(s);
            if ended {
                break;
            }
        }
        let tail = match (ended, value) {
            (false, Some(ty)) => {
                let e = self.full(ty, 0);
                stmts.extend(self.flush());
                match clamp {
                    None => Some(e),
                    Some(r) => {
                        let name = self.fresh("v");
                        let id = self.declare(name.clone(), ty.clone(), Kind::Val, None);
                        self.binds[id].read = true;
                        stmts.push(S::Bind(id, Some(ty.to_string()), e));
                        let v = E::name(&name);
                        stmts.push(S::Guard(
                            E::bin(
                                Op::And,
                                E::bin(Op::Ge, v.clone(), E::num(r.lo)),
                                E::bin(Op::Le, v.clone(), E::num(r.hi)),
                            ),
                            Block {
                                stmts: vec![S::Return(Some(self.default_of(ty, Some(r))))],
                                tail: None,
                            },
                        ));
                        Some(v)
                    }
                }
            }
            _ => None,
        };
        self.close(&mut stmts, ended);
        // Past the block a binding holds what it held before or what the block left in it, and
        // what a `guard` in the block told of it no longer holds.
        for (id, was) in before.into_iter().enumerate() {
            self.binds[id].known = match (was, self.binds[id].known) {
                (Some(a), Some(b)) => Some(a.hull(b)),
                _ => None,
            };
        }
        Block { stmts, tail }
    }

    /// Whether the function being generated may have to prove a fact that relates two values:
    /// its recursion's measure at the call against the one on entry, or two names a call
    /// passes to parameters its callee relates ([`Gen::arguments`]). Tier 1 relates the values
    /// of a function of at most `RELATIONAL_LIMIT` variables, and keeps each one's bounds
    /// alone past that (`octagon.rs`), so such a function stays small.
    fn relates(&self) -> bool {
        self.mode == Mode::Refined && (self.recursion.is_some() || self.small)
    }

    /// Ends a scope: each binding of it nobody read is discarded on purpose before the block
    /// leaves, and a `var` nobody assigned becomes a `val` when it is printed.
    fn close(&mut self, stmts: &mut Vec<S>, ended: bool) {
        let ids = self.scopes.pop().unwrap_or_default();
        let at = if ended {
            stmts.len().saturating_sub(1)
        } else {
            stmts.len()
        };
        for id in ids.into_iter().rev() {
            // A pattern's binding nobody read is written `_` in its pattern instead.
            if !self.binds[id].read && !matches!(self.binds[id].kind, Kind::Param | Kind::Pattern) {
                self.binds[id].read = true;
                stmts.insert(at, S::Discard(E::name(&self.binds[id].name)));
            }
        }
    }

    fn statement(&mut self, depth: u32) -> Vec<S> {
        self.cost += self.mult;
        let m = self.mark();
        let roll = self.rng.below(101);
        let mut out = match roll {
            0..=29 => self.binding(depth),
            30..=49 => self.assignment(),
            50..=53 if depth < 3 => self.if_statement(depth),
            54..=58 if depth < 3 => self.match_statement(depth),
            59..=68 if depth < 3 && self.loops < 2 => self.loop_statement(depth),
            69..=76 => self.guard_statement(),
            77..=85 => self.call_statement(),
            86..=88 => {
                let ty = self.ty(1);
                let e = self.value(&ty, 1);
                let mut out = self.flush();
                out.push(S::Discard(e));
                out
            }
            89..=94 if depth > 0 => {
                // A function that returns an array is rarer than the other places of an array
                // of no type, so its return takes a share of its own.
                let leave = match self.leaving_return() {
                    Some(s) => s,
                    None => self.leave(),
                };
                let mut out = self.flush();
                out.push(leave);
                out
            }
            95..=96 if depth > 0 => self.leaving_binding(),
            97..=98 if depth > 0 => self.leaving_value(),
            99..=100 if depth > 0 => self.leaving_aggregate(),
            _ => self.binding(depth),
        };
        if out.is_empty() {
            self.restore(m);
            out = self.binding(depth);
        }
        out
    }

    /// `if c { return a } else { return b }`, a value whose every branch leaves, in a function
    /// with a result type; none in one without.
    fn leaving_if(&mut self) -> Option<E> {
        let t = self.current.as_ref().and_then(|s| s.ret.clone())?;
        let r = self.current.as_ref().and_then(|s| s.ret_range);
        let cond = self.bool_expr(1);
        let a = E::Return(Box::new(self.default_of(&t, r)));
        let b = E::Return(Box::new(self.default_of(&t, r)));
        Some(E::If(Box::new(cond), Box::new(a), Box::new(b)))
    }

    /// Which bindings count as read, to compare with after a value is generated.
    fn reads(&self) -> Vec<bool> {
        self.binds.iter().map(|b| b.read).collect()
    }

    /// Takes back the reads counted since `before` that `kept` and the pending guards do not
    /// hold: a part of a generated value was replaced, and a binding only that part read is
    /// still unread, so its scope discards it when it closes (`E0218`).
    fn forget_reads(&mut self, before: &[bool], kept: &E) {
        for (id, was) in before.iter().enumerate() {
            if *was || !self.binds[id].read {
                continue;
            }
            let name = [self.binds[id].name.clone()];
            let held = Gen::reads_any(kept, &name)
                || self.pending.iter().any(|c| Gen::reads_any(c, &name));
            if !held {
                self.binds[id].read = false;
            }
        }
    }

    /// A statement whose value holds a value whose every branch leaves, as an operand, an
    /// element or an argument, so that it leaves where it stands (`core-calculus.md`, item 6).
    fn leaving_value(&mut self) -> Vec<S> {
        let Some(leave) = self.leaving_if() else {
            return Vec::new();
        };
        let s = match self.rng.below(3) {
            0 => {
                let x = self.value(&Ty::Int(U8), 1);
                S::Discard(E::bin(Op::Add, x, leave))
            }
            1 if self.rng.chance(50) => S::Discard(E::Array(vec![leave])),
            1 => {
                let x = self.value(&Ty::Int(U8), 1);
                S::Discard(E::Array(vec![leave, x]))
            }
            _ => {
                let sigs: Vec<Sig> = self
                    .sigs
                    .iter()
                    .filter(|s| {
                        s.params.iter().any(|p| !p.is_mut)
                            && self.cost + self.mult * s.cost <= COST_BUDGET
                    })
                    .cloned()
                    .collect();
                if sigs.is_empty() {
                    return Vec::new();
                }
                let sig = self.rng.pick(&sigs).clone();
                let read = self.reads();
                let Some(E::Call(name, mut args)) = self.arguments(&sig, false, 0) else {
                    return Vec::new();
                };
                let Some(k) = sig.params.iter().position(|p| !p.is_mut) else {
                    return Vec::new();
                };
                // No argument names a place the call passes as `mut` (`E0207`), and the value
                // that leaves was generated before the call chose its places.
                let passed: Vec<String> = args
                    .iter()
                    .filter(|(is_mut, _)| *is_mut)
                    .filter_map(|(_, a)| self.root(a))
                    .map(|id| self.binds[id].name.clone())
                    .collect();
                if Gen::reads_any(&leave, &passed) {
                    return Vec::new();
                }
                args[k].1 = leave;
                self.cost += self.mult * sig.cost;
                let call = E::Call(name, args);
                self.forget_reads(&read, &call);
                match sig.ret {
                    Some(_) => S::Discard(call),
                    None => S::Expr(call),
                }
            }
        };
        let mut out = self.flush();
        out.push(s);
        out
    }

    /// An array of `n` elements of type `elem` of which each gives no value: `[leave]`,
    /// `[leave; n]`, or such an array of such arrays (`core-calculus.md`, item 6). None for no
    /// elements, whose array is the value `[]`, or in a function without a result type.
    fn no_type_array(&mut self, n: u32, elem: &Ty) -> Option<E> {
        if n == 0 {
            return None;
        }
        let x = match elem {
            Ty::Array(m, inner) if *m > 0 && self.rng.chance(30) => {
                self.no_type_array(*m, inner)?
            }
            _ => self.leaving_if()?,
        };
        Some(if n <= 3 && self.rng.chance(60) {
            E::Array(vec![x; n as usize])
        } else {
            E::Repeat(Box::new(x), n)
        })
    }

    /// Now and then, in a function that returns an array, `return [leave]`: an array of no type
    /// as a returned value (`core-calculus.md`, item 6).
    fn leaving_return(&mut self) -> Option<S> {
        let Some(Ty::Array(n, elem)) = self.current.as_ref().and_then(|s| s.ret.clone()) else {
            return None;
        };
        if n == 0 || !self.rng.chance(40) {
            return None;
        }
        Some(S::Return(Some(self.no_type_array(n, &elem)?)))
    }

    /// A statement that holds an array of no type where a value stands: as a returned value, an
    /// assigned value, a struct's or a variant's field, an element beside a value, or the value
    /// of a repeat of no copies, so that it leaves where it stands (`core-calculus.md`, item 6).
    fn leaving_aggregate(&mut self) -> Vec<S> {
        let Some(s) = self.aggregate_statement() else {
            return Vec::new();
        };
        let mut out = self.flush();
        out.push(s);
        out
    }

    /// The array types of at least one element that `t` holds, itself included, through arrays
    /// and struct fields, as deep as a place reaches.
    fn array_types(&self, t: &Ty, depth: usize, out: &mut Vec<Ty>) {
        if depth >= 4 {
            return;
        }
        match t {
            Ty::Array(n, inner) if *n > 0 => {
                out.push(t.clone());
                self.array_types(inner, depth + 1, out);
            }
            Ty::Struct(name) => {
                for f in self.fields_of(name) {
                    if f.refine.is_none() {
                        self.array_types(&f.ty, depth + 1, out);
                    }
                }
            }
            Ty::Array(..) | Ty::Int(_) | Ty::Bool | Ty::Enum(_) => {}
        }
    }

    /// The statement [`Gen::leaving_aggregate`] adds, at a position the function and its scope
    /// have: none in a function without a result type.
    fn aggregate_statement(&mut self) -> Option<S> {
        let array_field = |fields: &[GField]| -> Vec<usize> {
            (0..fields.len())
                .filter(|&k| matches!(fields[k].ty, Ty::Array(n, _) if n > 0))
                .collect()
        };
        self.current.as_ref().and_then(|s| s.ret.as_ref())?;
        let ret = self.current.as_ref().and_then(|s| s.ret.clone());
        // The array types of the places a statement may assign.
        let mut places = Vec::new();
        for id in self.visible() {
            if matches!(self.binds[id].kind, Kind::Var | Kind::MutParam)
                && !self.frozen.contains(&id)
            {
                self.array_types(&self.binds[id].ty.clone(), 0, &mut places);
            }
        }
        let has_field = self
            .structs
            .iter()
            .any(|s| !array_field(&s.fields).is_empty());
        let has_payload = self
            .enums
            .iter()
            .any(|e| e.variants.iter().any(|(_, f)| !array_field(f).is_empty()));
        // The places a function or a program must have a type for come up three times each,
        // since few have one; the two any function has, once.
        let kinds: Vec<u32> = [
            (matches!(ret, Some(Ty::Array(n, _)) if n > 0), 3),
            (!places.is_empty(), 3),
            (has_field, 3),
            (has_payload, 3),
            (true, 1),
            (true, 1),
        ]
        .iter()
        .enumerate()
        .filter(|(_, (on, _))| *on)
        .flat_map(|(k, (_, times))| std::iter::repeat_n(k as u32, *times))
        .collect();
        match *self.rng.pick(&kinds) {
            0 => {
                let Some(Ty::Array(n, elem)) = ret else {
                    return None;
                };
                Some(S::Return(Some(self.no_type_array(n, &elem)?)))
            }
            1 => {
                let t = self.rng.pick(&places).clone();
                let (id, place) = self.place(Some(&t))?;
                let Ty::Array(n, elem) = t else {
                    return None;
                };
                let value = self.no_type_array(n, &elem)?;
                self.binds[id].assigned = true;
                Some(S::Assign(place, "=", value))
            }
            2 => {
                let names: Vec<String> = self
                    .structs
                    .iter()
                    .filter(|s| !array_field(&s.fields).is_empty())
                    .map(|s| s.name.clone())
                    .collect();
                if names.is_empty() {
                    return None;
                }
                let name = self.rng.pick(&names).clone();
                let fields = self.fields_of(&name);
                let k = *self.rng.pick(&array_field(&fields));
                let Ty::Array(n, elem) = &fields[k].ty else {
                    return None;
                };
                let leave = self.no_type_array(*n, elem)?;
                let read = self.reads();
                let E::Construct(_, mut values) = self.construct(&name, 1) else {
                    return None;
                };
                values[k].1 = leave;
                let built = E::Construct(name, values);
                self.forget_reads(&read, &built);
                Some(S::Discard(built))
            }
            3 => {
                let variants: Vec<(String, String, Vec<GField>)> = self
                    .enums
                    .iter()
                    .flat_map(|e| {
                        e.variants
                            .iter()
                            .filter(|(_, fields)| !array_field(fields).is_empty())
                            .map(|(v, fields)| (e.name.clone(), v.clone(), fields.clone()))
                    })
                    .collect();
                if variants.is_empty() {
                    return None;
                }
                let (name, v, fields) = self.rng.pick(&variants).clone();
                let k = *self.rng.pick(&array_field(&fields));
                let mut values = Vec::new();
                for (j, f) in fields.iter().enumerate() {
                    let x = match (&f.ty, f.refine) {
                        (Ty::Array(n, elem), _) if j == k => self.no_type_array(*n, elem)?,
                        (Ty::Int(_), Some(r)) => self.refined_value(r),
                        (Ty::Array(n, _), Some(r)) => self.refined_array(*n, r),
                        (ty, _) => self.value(ty, 1),
                    };
                    values.push((f.name.clone(), x));
                }
                Some(S::Discard(E::Variant(name, v, values)))
            }
            4 => {
                // An array of no type beside arrays of a type: `[[x], [leave]]`.
                let n = 1 + self.rng.below(2) as u32;
                let inner = Ty::Array(n, Box::new(Ty::Int(U8)));
                let len = 1 + self.rng.below(3) as usize;
                let k = self.rng.below(len as u64) as usize;
                // The arrays of a type beside it share one machine type, as the elements of
                // any array literal do (`E0222`): the array of no type holds no integer.
                self.hulls.push(None);
                let mut items = Vec::new();
                for j in 0..len {
                    items.push(if j == k {
                        self.no_type_array(n, &Ty::Int(U8))
                    } else {
                        Some(self.value(&inner, 1))
                    });
                }
                let hull = self.hulls.pop().flatten();
                let items: Vec<E> = items.into_iter().collect::<Option<_>>()?;
                if hull.is_some_and(|h| !h.fits()) {
                    return None;
                }
                Some(S::Discard(E::Array(items)))
            }
            _ => {
                // A repeat of no copies evaluates its value once, and leaves where it does.
                let value = if self.rng.chance(50) {
                    self.leaving_if()?
                } else {
                    self.no_type_array(1, &Ty::Int(U8))?
                };
                let repeat = E::Repeat(Box::new(value), 0);
                Some(S::Discard(if self.rng.chance(50) {
                    E::Len(Box::new(repeat))
                } else {
                    repeat
                }))
            }
        }
    }

    /// `val v: T = if c { return a } else { return b }`, then a read of `v` that never runs: a
    /// binding whose initializer's every branch leaves, so that it has no value
    /// (`core-calculus.md`, item 6). None in a function without a result type.
    fn leaving_binding(&mut self) -> Vec<S> {
        let Some(init) = self.leaving_if() else {
            return Vec::new();
        };
        let ty = self.ty(1);
        let mut out = self.flush();
        let name = self.fresh("v");
        let id = self.declare(name.clone(), ty.clone(), Kind::Val, None);
        self.binds[id].read = true;
        out.push(S::Bind(id, Some(ty.to_string()), init));
        out.push(S::Discard(E::name(&name)));
        out
    }

    fn binding(&mut self, depth: u32) -> Vec<S> {
        if self.rng.chance(20) {
            let m = self.mark();
            if let Some((ty, init)) = self.inferred(depth) {
                let kind = if self.rng.chance(50) {
                    Kind::Var
                } else {
                    Kind::Val
                };
                let mut out = self.flush();
                let name = self.fresh("v");
                let id = self.declare(name, ty, kind, None);
                self.initialized(id, &init);
                out.push(S::Bind(id, None, init));
                return out;
            }
            self.restore(m);
        }
        let ty = self.ty(0);
        let mut range = None;
        let mut ty_text = ty.to_string();
        let m = self.mark();
        let init = match &ty {
            Ty::Int(t) if self.mode == Mode::Refined && self.rng.chance(35) => {
                match self.refined_init(*t) {
                    Some((e, r)) => {
                        ty_text = format!("{}[{}..={}]", t.name, r.lo, r.hi);
                        range = Some(r);
                        e
                    }
                    None => {
                        self.restore(m);
                        self.full(&ty, depth)
                    }
                }
            }
            Ty::Int(t) if self.guarded() && self.rng.chance(30) => {
                match self.guarded_narrowing(*t) {
                    Some(e) => e,
                    None => {
                        self.restore(m);
                        self.full(&ty, depth)
                    }
                }
            }
            _ => self.full(&ty, depth),
        };
        let kind = if range.is_none() && self.rng.chance(50) {
            Kind::Var
        } else {
            Kind::Val
        };
        let mut out = self.flush();
        let name = self.fresh("v");
        let id = self.declare(name, ty, kind, range);
        self.initialized(id, &init);
        out.push(S::Bind(id, Some(ty_text), init));
        out
    }

    /// The initializer of a binding that states no type, with the type it takes
    /// (`core-calculus.md` item 5): its integer operands' one type, `i64` for literals alone, or
    /// `bool`. None when the operands have several types, which needs a stated type (`E0212`),
    /// or when the value cannot land in the type.
    fn inferred(&mut self, depth: u32) -> Option<(Ty, E)> {
        if self.rng.chance(25) {
            return Some((Ty::Bool, self.bool_expr(depth)));
        }
        if !self.structs.is_empty() && self.rng.chance(20) {
            let k = self.rng.below(self.structs.len() as u64) as usize;
            let name = self.structs[k].name.clone();
            return Some((Ty::Struct(name.clone()), self.construct(&name, depth)));
        }
        let g = self.int_expr(Want::Any, depth);
        let t = match g.cls {
            Cls::One(t) => t,
            Cls::Lit => I64,
            Cls::Mixed(_) => return None,
        };
        (g.r.meets(t.range()) && g.k.is_none_or(|v| t.range().holds(v)) && self.may_fit(&g.e, t))
            .then_some((Ty::Int(t), g.e))
    }

    /// `x + k` or `x - k`, guarded so that it fits `t`; no guard when `x`'s range already makes
    /// it fit.
    fn guarded_narrowing(&mut self, t: IntT) -> Option<E> {
        let (x, r, _) = self.int_name(None, false)?;
        let k = Num::new(false, 1 + u128::from(self.rng.below(20)));
        let (op, guard, limit) = if self.rng.chance(50) {
            (Op::Add, Op::Le, t.max().sub(k)?)
        } else {
            (Op::Sub, Op::Ge, t.min().add(k)?)
        };
        let fits = match op {
            Op::Add => r.hi <= limit,
            _ => r.lo >= limit,
        };
        if !fits {
            self.pending.push(E::bin(guard, x.clone(), E::num(limit)));
        }
        Some(E::bin(op, x, E::num(k)))
    }

    /// An initializer Tier 1 proves inside a refinement of `t`, and the refinement.
    fn refined_init(&mut self, t: IntT) -> Option<(E, R)> {
        match self.rng.below(3) {
            0 if !self.safe.is_empty() => {
                let (e, max) = self.rng.pick(&self.safe).clone();
                let r = R {
                    lo: Num::ZERO,
                    hi: Num::new(false, max),
                };
                (t.range().holds(r.hi)).then_some((e, r))
            }
            1 => {
                let (x, ..) = self.int_name(None, true)?;
                let k = 1 + u128::from(self.rng.below(16));
                let range = R {
                    lo: Num::ZERO,
                    hi: Num::new(false, k - 1),
                };
                Some((E::bin(Op::Rem, x, E::Lit(k.to_string())), range))
            }
            _ => {
                let (x, ..) = self.int_name(None, false)?;
                let lo = self.value_in(t.range());
                let width = Num::new(false, u128::from(self.rng.below(200)));
                let hi = lo.add(width)?.min(t.max());
                let guard = E::bin(
                    Op::And,
                    E::bin(Op::Ge, x.clone(), E::num(lo)),
                    E::bin(Op::Le, x.clone(), E::num(hi)),
                );
                self.pending.push(guard);
                Some((x, R { lo, hi }))
            }
        }
    }

    fn assignment(&mut self) -> Vec<S> {
        let Some((id, place)) = self.place(None) else {
            return Vec::new();
        };
        let ty = self.place_ty(id, &place);
        let refine = self.place_refine(id, &place);
        let (op, value) = match (&ty, refine) {
            // A refined field takes a value proven inside its refinement, and an array whose
            // elements are refined, elements proven so.
            (Ty::Int(_), Some(r)) => ("=", self.refined_value(r)),
            (Ty::Array(n, _), Some(r)) => ("=", self.refined_array(*n, r)),
            (Ty::Int(t), None) if self.rng.chance(50) => {
                let place_range = match &place {
                    E::Name(_) => self.binds[id].range.unwrap_or(t.range()),
                    _ => t.range(),
                };
                match self.rng.below(10) {
                    0..=5 => {
                        let op = *self
                            .rng
                            .pick(&[Op::Add, Op::Sub, Op::Mul, Op::Div, Op::Rem]);
                        let mut v = self.int_expr(Want::Any, 1);
                        if matches!(op, Op::Div | Op::Rem) {
                            v = self.divisor(v);
                        }
                        let fits = t.bits == 128
                            || v.cls.wide().is_some()
                            || arithmetic(op, place_range, v.r)
                                .is_some_and(|r| r.hull(place_range).hull(v.r).fits());
                        if !fits {
                            return Vec::new();
                        }
                        (compound(op), v.e)
                    }
                    6..=7 => {
                        let op = *self.rng.pick(&[Op::BitAnd, Op::BitOr, Op::BitXor]);
                        (compound(op), self.int_expr(Want::Exact(*t), 1).e)
                    }
                    _ => {
                        let op = if self.rng.chance(50) {
                            Op::Shl
                        } else {
                            Op::Shr
                        };
                        let k = u128::from(self.rng.next()) % u128::from(t.bits);
                        (compound(op), E::Lit(k.to_string()))
                    }
                }
            }
            _ => ("=", self.full(&ty, 1)),
        };
        if let (E::Name(_), Ty::Int(t)) = (&place, &ty) {
            let now = self.known_range(id, *t);
            let then = match op {
                "=" => self.fact(&value),
                "+=" => self.fact(&value).and_then(|v| arithmetic(Op::Add, now, v)),
                "-=" => self.fact(&value).and_then(|v| arithmetic(Op::Sub, now, v)),
                "*=" => self.fact(&value).and_then(|v| arithmetic(Op::Mul, now, v)),
                "<<=" => constant(&value)
                    .and_then(|k| 1u128.checked_shl(u32::try_from(k.mag).ok()?))
                    .and_then(|p| arithmetic(Op::Mul, now, R::point(Num::new(false, p)))),
                _ => None,
            };
            // A value known to lie wholly outside the place's type always traps (`E0508`).
            if then.is_some_and(|r| !r.meets(t.range())) {
                return Vec::new();
            }
            self.binds[id].known = then.and_then(|r| r.meet(t.range()));
        }
        self.binds[id].assigned = true;
        let mut out = self.flush();
        out.push(S::Assign(place, op, value));
        out
    }

    /// An `if`, now and then with `else if`s: each condition of the chain is generated first, so
    /// that what they need is guarded before the whole statement.
    fn if_statement(&mut self, depth: u32) -> Vec<S> {
        let mut conds = vec![self.bool_expr(0)];
        while conds.len() < 4 && self.rng.chance(25) {
            conds.push(self.bool_expr(0));
        }
        let mut out = self.flush();
        let mut els = self
            .rng
            .chance(50)
            .then(|| self.block(depth + 1, None, Vec::new(), None));
        while let Some(cond) = conds.pop() {
            let then = self.block(depth + 1, None, Vec::new(), None);
            let s = S::If(cond, then, els);
            if conds.is_empty() {
                out.push(s);
                break;
            }
            els = Some(Block {
                stmts: vec![s],
                tail: None,
            });
        }
        out
    }

    fn loop_statement(&mut self, depth: u32) -> Vec<S> {
        let up = self.rng.chance(70);
        let t = *self.rng.pick(&[
            INTS[0], INTS[1], INTS[2], INTS[3], INTS[5], INTS[5], INTS[6], INTS[7], INTS[8],
            INTS[9],
        ]);
        let bound: u64 = if self.rng.chance(5) {
            0
        } else {
            1 + self.rng.below(5)
        };
        if self.cost + self.mult * bound.max(1) * 4 > COST_BUDGET {
            return Vec::new();
        }
        // A condition beside the counter's, which reads no counter and calls nothing.
        let more = self.rng.chance(20).then(|| {
            self.no_calls = true;
            let more = self.bool_expr(1);
            self.no_calls = false;
            more
        });
        let name = self.fresh("c");
        let c = E::name(&name);
        let init = if up { 0 } else { bound };
        let id = self.declare(name, Ty::Int(t), Kind::Counter, None);
        self.binds[id].read = true;
        self.binds[id].assigned = true;
        let mut cond = if up {
            E::bin(Op::Lt, c.clone(), E::Lit(bound.to_string()))
        } else {
            E::bin(Op::Gt, c.clone(), E::Lit("0".into()))
        };
        if let Some(more) = more {
            cond = E::bin(Op::And, cond, more);
        }
        let mut clauses = Vec::new();
        let invariant = self.mode == Mode::Refined && self.rng.chance(50);
        if self.mode == Mode::Refined {
            if invariant {
                // A signed counter needs its lower bound written: the loop's text gives none.
                clauses.push(if t.signed {
                    format!("invariant {0} >= 0 and {0} <= {1}", c.text(), bound)
                } else {
                    format!("invariant {} <= {}", c.text(), bound)
                });
            }
            if self.rng.chance(60) {
                clauses.push(if up {
                    format!("decreases {} - {}", bound, c.text())
                } else {
                    format!("decreases {}", c.text())
                });
            }
        }
        let mut out = self.flush();
        out.push(S::Bind(
            id,
            Some(t.name.to_string()),
            E::Lit(init.to_string()),
        ));
        let step = S::Assign(c.clone(), if up { "+=" } else { "-=" }, E::Lit("1".into()));
        // After the step the counter lies in 1 ..= bound going up, 0 ..= bound - 1 going down, as
        // far as Tier 1 can tell: the loop's condition bounds it on one side only, its type or an
        // invariant on the other.
        let saved = self.safe.len();
        if bound > 0 && (invariant || (up && !t.signed)) {
            let max = u128::from(bound) - 1;
            if up {
                self.safe
                    .push((E::bin(Op::Sub, c.clone(), E::Lit("1".into())), max));
                self.safe.push((c.clone(), max + 1));
            } else {
                self.safe.push((c.clone(), max));
            }
        }
        self.loops += 1;
        let mult = self.mult;
        self.mult *= bound.max(1);
        // In the body, after the step, and then after the loop.
        let most = Num::new(false, u128::from(bound));
        let one = Num::small(1);
        self.binds[id].known = if up {
            Some(R { lo: one, hi: most })
        } else {
            most.sub(one).map(|hi| R { lo: Num::ZERO, hi })
        }
        .filter(|r| r.lo <= r.hi);
        let body = self.block(depth + 1, None, vec![step], None);
        self.binds[id].known = Some(R {
            lo: Num::ZERO,
            hi: most,
        });
        self.mult = mult;
        self.loops -= 1;
        self.safe.truncate(saved);
        out.push(S::While(cond, clauses, body));
        out
    }

    fn guard_statement(&mut self) -> Vec<S> {
        let named = self.guarded() && self.rng.chance(70);
        let cond = match named.then(|| self.int_name(None, false)).flatten() {
            Some((x, r, _)) => {
                let op = *self.rng.pick(&[Op::Lt, Op::Le, Op::Gt, Op::Ge]);
                let n = self.value_in(r);
                E::bin(op, x, E::num(n))
            }
            _ => self.bool_expr(0),
        };
        self.pending.push(cond);
        self.flush()
    }

    fn call_statement(&mut self) -> Vec<S> {
        let units: Vec<Sig> = self
            .sigs
            .iter()
            .filter(|s| s.ret.is_none() && self.cost + self.mult * s.cost <= COST_BUDGET)
            .cloned()
            .collect();
        let s = if !units.is_empty() && self.rng.chance(60) {
            let sig = self.rng.pick(&units).clone();
            let m = self.mark();
            let Some(e) = self.arguments(&sig, false, 0) else {
                self.restore(m);
                return Vec::new();
            };
            self.cost += self.mult * sig.cost;
            S::Expr(e)
        } else {
            match self.call(0, |_| true) {
                Some((e, _)) => S::Discard(e),
                None => return Vec::new(),
            }
        };
        let mut out = self.flush();
        out.push(s);
        out
    }

    // ---------------------------------------------------------------- functions

    fn function(&mut self, index: usize, public: bool) -> String {
        self.binds.clear();
        self.exclude.clear();
        self.pending.clear();
        self.scopes = vec![Vec::new()];
        self.names = 0;
        self.cost = 0;
        self.mult = 1;
        self.loops = 0;
        self.recursed = false;
        self.safe.clear();
        self.rng = stream(self.seed, index, 0);
        let name = format!("f{}", index);
        let refined = self.mode == Mode::Refined;
        let mut params: Vec<Param> = Vec::new();
        self.recursion = self
            .rng
            .chance(if refined { 20 } else { 10 })
            .then(|| 2 + self.rng.below(6));
        // Now and then a refined function that does not recurse stays small too, so that its
        // calls may relate two names.
        self.small = refined && self.recursion.is_none() && self.rng.chance(50);
        if let Some(depth) = self.recursion {
            params.push(Param {
                name: "n".into(),
                is_mut: false,
                ty: Ty::Int(U8),
                refine: refined.then_some(Refine {
                    lo: Bnd::Lit(Num::ZERO),
                    hi: Bnd::Lit(Num::new(false, u128::from(depth))),
                    inclusive: true,
                }),
            });
        }
        for _ in 0..self.rng.below(5) {
            let k = params.len();
            // Now and then, in a refined program, an index into an array parameter before it:
            // an unsigned integer below that array's length.
            let arrays: Vec<usize> = (0..params.len())
                .filter(|&i| matches!(params[i].ty, Ty::Array(n, _) if n > 0))
                .collect();
            if refined && !arrays.is_empty() && self.rng.chance(25) {
                let t = *self
                    .rng
                    .pick(&[INTS[0], INTS[1], INTS[2], INTS[5], INTS[5]]);
                params.push(Param {
                    name: format!("p{}", k),
                    is_mut: false,
                    ty: Ty::Int(t),
                    refine: Some(Refine {
                        lo: Bnd::Lit(Num::ZERO),
                        hi: Bnd::Len(*self.rng.pick(&arrays)),
                        inclusive: false,
                    }),
                });
                continue;
            }
            let ty = self.ty(0);
            let is_mut = self.rng.chance(25);
            let refine = match (&ty, refined && !is_mut && self.rng.chance(40)) {
                (Ty::Int(t), true) => self.param_refinement(*t, &params),
                _ => None,
            };
            params.push(Param {
                name: format!("p{}", k),
                is_mut,
                ty,
                refine,
            });
        }
        // Now and then a function that changes an array of structs and gives an index, which
        // a caller holding such an array may index it with (`index_call`).
        let changer = !self.structs.is_empty() && self.rng.chance(30);
        if changer {
            let k = self.rng.below(self.structs.len() as u64) as usize;
            let name = self.structs[k].name.clone();
            params.push(Param {
                name: format!("p{}", params.len()),
                is_mut: true,
                ty: Ty::Array(2, Box::new(Ty::Struct(name))),
                refine: None,
            });
        }
        let ret = if changer {
            Some(Ty::Int(USIZE))
        } else {
            self.rng.chance(80).then(|| self.ty(0))
        };
        let mut ret_refined = false;
        let mut ensures = Vec::new();
        let mut ret_range = None;
        if let (Some(Ty::Int(t)), true) = (&ret, refined) {
            if self.rng.chance(45) {
                let lo = self.value_in(R {
                    lo: t.min().max(Num::small(-100)),
                    hi: t.max().min(Num::small(100)),
                });
                let hi = lo
                    .add(Num::new(false, u128::from(self.rng.below(300))))
                    .map_or(t.max(), |h| h.min(t.max()));
                ret_range = Some(R { lo, hi });
                if self.rng.chance(50) {
                    ret_refined = true;
                } else {
                    ensures.push(format!("ensures result >= {} and result <= {}", lo, hi));
                }
            }
        }
        let requires = if refined && self.rng.chance(45) {
            self.requires(&params)
        } else {
            Vec::new()
        };
        for p in &params {
            let range = p.refine.as_ref().and_then(|r| match (&r.lo, &r.hi) {
                (Bnd::Lit(lo), Bnd::Lit(hi)) => Some(R {
                    lo: *lo,
                    hi: if r.inclusive {
                        *hi
                    } else {
                        hi.sub(Num::small(1))?
                    },
                }),
                _ => None,
            });
            let kind = if p.is_mut {
                Kind::MutParam
            } else {
                Kind::Param
            };
            let id = self.declare(p.name.clone(), p.ty.clone(), kind, range);
            self.binds[id].read = true;
        }
        for r in &requires {
            if let Req::Below(a, b) = *r {
                if let Ty::Array(n, _) = &params[b].ty {
                    if *n > 0 {
                        self.safe
                            .push((E::name(&params[a].name), u128::from(*n) - 1));
                    }
                }
            }
        }
        let sig = Sig {
            name: name.clone(),
            params: params.clone(),
            ret: ret.clone(),
            ret_range,
            requires: requires.clone(),
            cost: 0,
        };
        for r in &requires {
            if let Req::Bound(a, op, c) = *r {
                self.learn(&E::bin(op, E::name(&params[a].name), E::num(c)));
            }
        }
        self.current = Some(sig.clone());
        // The body, of this attempt: another attempt has another body under the same signature.
        let attempt = self.attempts.get(index).copied().unwrap_or(0);
        self.rng = stream(self.seed, index, 1 + u64::from(attempt));
        let mut first = Vec::new();
        if self.recursion.is_some() {
            let leave = S::Return(ret.as_ref().map(|t| self.default_of(t, ret_range)));
            first.push(S::Guard(
                E::bin(Op::Gt, E::name("n"), E::Lit("0".into())),
                Block {
                    stmts: vec![leave.clone()],
                    tail: None,
                },
            ));
            if !refined {
                first.push(S::Guard(
                    E::bin(
                        Op::Le,
                        E::name("n"),
                        E::Lit(self.recursion.unwrap_or(1).to_string()),
                    ),
                    Block {
                        stmts: vec![leave],
                        tail: None,
                    },
                ));
            }
            for s in &first {
                if let S::Guard(c, _) = s {
                    self.learn(&c.clone());
                }
            }
        }
        // An expression body for a small function without contracts on its result.
        let expression = first.is_empty() && ret_range.is_none() && self.rng.chance(20);
        let body = match (&ret, expression) {
            (Some(t), true) => {
                // Now and then the body is itself a value whose every branch leaves.
                let leaving = if self.rng.chance(8) {
                    self.leaving_if()
                } else {
                    None
                };
                let mut e = match leaving {
                    Some(e) => e,
                    None => self.full(t, 0),
                };
                self.returns_in(&mut e, t);
                if self.pending.is_empty() {
                    Body::Expr(e)
                } else {
                    Body::Block(Block {
                        stmts: self.flush(),
                        tail: Some(e),
                    })
                }
            }
            _ => Body::Block(self.block(0, ret.as_ref(), first, ret_range)),
        };
        let mut cost = self.cost.max(1);
        if let Some(depth) = self.recursion {
            cost *= depth + 1;
        }
        self.sigs.push(Sig { cost, ..sig });
        self.current = None;

        // The text.
        let mut out = String::new();
        if public {
            out.push_str("pub ");
        }
        out.push_str(&format!("fn {}(", name));
        for (k, p) in params.iter().enumerate() {
            if k > 0 {
                out.push_str(", ");
            }
            if p.is_mut {
                out.push_str("mut ");
            }
            out.push_str(&format!("{}: {}", p.name, p.ty));
            if let Some(r) = &p.refine {
                let b = |b: &Bnd| match b {
                    Bnd::Lit(n) => n.to_string(),
                    Bnd::Name(i) => params[*i].name.clone(),
                    Bnd::Len(i) => format!("{}.len", params[*i].name),
                };
                out.push_str(&format!(
                    "[{}{}{}]",
                    b(&r.lo),
                    if r.inclusive { "..=" } else { ".." },
                    b(&r.hi)
                ));
            }
        }
        out.push(')');
        if let Some(t) = &ret {
            out.push_str(&format!(" -> {}", t));
            if let (true, Some(r)) = (ret_refined, ret_range) {
                out.push_str(&format!("[{}..={}]", r.lo, r.hi));
            }
        }
        let mut clauses: Vec<String> = requires
            .iter()
            .map(|r| format!("requires {}", req_text(r, &params)))
            .collect();
        clauses.extend(ensures);
        if self.recursion.is_some() && refined {
            clauses.push("decreases n".into());
        }
        for c in &clauses {
            out.push_str(&format!("\n  {}", c));
        }
        match body {
            Body::Expr(e) => {
                out.push_str(if clauses.is_empty() { " = " } else { "\n= " });
                out.push_str(&e.text());
                out.push('\n');
            }
            Body::Block(b) => {
                out.push_str(if clauses.is_empty() { " {\n" } else { "\n{\n" });
                self.print_block(&b, 1, &mut out);
                out.push_str("}\n");
            }
        }
        out
    }

    fn param_refinement(&mut self, t: IntT, before: &[Param]) -> Option<Refine> {
        let arrays: Vec<usize> = (0..before.len())
            .filter(|&i| matches!(before[i].ty, Ty::Array(..)))
            .collect();
        let names: Vec<usize> = (0..before.len())
            .filter(|&i| matches!(before[i].ty, Ty::Int(_)) && !before[i].is_mut)
            .collect();
        // Now and then a 64-bit parameter whose values all lie near 2^62 or 2^63, or from
        // 2^63 up: the product of two such lies wholly past Tier 1's domain
        // ([`Gen::near_edge`]).
        if t.bits == 64 && self.rng.chance(12) {
            let half = 1u128 << 62;
            let (lo, hi) = match (t.signed, self.rng.below(3)) {
                (false, 0) => (2 * half, t.max().mag),
                (_, 1) => (half, half + u128::from(self.rng.below(1000))),
                _ => (
                    2 * half - 1 - u128::from(self.rng.below(1000)),
                    2 * half - 1,
                ),
            };
            let down = t.signed && self.rng.chance(30);
            let (lo, hi) = if down {
                (Num::new(true, hi), Num::new(true, lo))
            } else {
                (Num::new(false, lo), Num::new(false, hi))
            };
            return Some(Refine {
                lo: Bnd::Lit(lo),
                hi: Bnd::Lit(hi),
                inclusive: true,
            });
        }
        match self.rng.below(10) {
            0..=3 if !arrays.is_empty() && !t.signed => Some(Refine {
                lo: Bnd::Lit(Num::ZERO),
                hi: Bnd::Len(*self.rng.pick(&arrays)),
                inclusive: false,
            }),
            4..=6 if !names.is_empty() && !t.signed => Some(Refine {
                lo: Bnd::Lit(Num::ZERO),
                hi: Bnd::Name(*self.rng.pick(&names)),
                inclusive: self.rng.chance(50),
            }),
            _ => {
                let lo = self.value_in(R {
                    lo: t.min().max(Num::small(-50)),
                    hi: t.max().min(Num::small(50)),
                });
                let hi = lo
                    .add(Num::new(false, 1 + u128::from(self.rng.below(100))))?
                    .min(t.max());
                (lo < hi).then_some(Refine {
                    lo: Bnd::Lit(lo),
                    hi: Bnd::Lit(hi),
                    inclusive: self.rng.chance(50),
                })
            }
        }
    }

    fn requires(&mut self, params: &[Param]) -> Vec<Req> {
        let ints: Vec<usize> = (0..params.len())
            .filter(|&i| matches!(params[i].ty, Ty::Int(_)) && !params[i].is_mut)
            .collect();
        let arrays: Vec<usize> = (0..params.len())
            .filter(|&i| matches!(params[i].ty, Ty::Array(n, _) if n > 0) && !params[i].is_mut)
            .collect();
        let mut out = Vec::new();
        for _ in 0..1 + self.rng.below(2) {
            if ints.is_empty() {
                break;
            }
            let a = *self.rng.pick(&ints);
            let Ty::Int(ta) = params[a].ty else { continue };
            let r = match self.rng.below(4) {
                0 if !arrays.is_empty() && !ta.signed => Req::Below(a, *self.rng.pick(&arrays)),
                1 if ints.len() > 1 => {
                    let b = *self.rng.pick(&ints);
                    if a == b {
                        continue;
                    }
                    Req::Order(a, *self.rng.pick(&[Op::Lt, Op::Le]), b)
                }
                2 if ints.len() > 1 => {
                    let b = *self.rng.pick(&ints);
                    if a == b {
                        continue;
                    }
                    let c = self.value_in(R {
                        lo: Num::ZERO,
                        hi: Num::small(1000),
                    });
                    Req::Sum(a, b, c)
                }
                _ => {
                    let op = *self.rng.pick(&[Op::Lt, Op::Le, Op::Gt, Op::Ge]);
                    // A bound Tier 1's domain holds, with room to double it: it proves no
                    // comparison with a constant at or beyond its limit, not even a literal's,
                    // so no call could meet the clause.
                    let reach = Num::new(false, (mente_sema::octagon::BIG / 4).unsigned_abs());
                    let c = self.value_in(R {
                        lo: ta.min().max(reach.negated()),
                        hi: ta.max().min(reach),
                    });
                    Req::Bound(a, op, c)
                }
            };
            out.push(r);
        }
        out
    }

    // ---------------------------------------------------------------- printing

    fn print_block(&self, b: &Block, indent: usize, out: &mut String) {
        for s in &b.stmts {
            self.print_stmt(s, indent, out);
        }
        if let Some(t) = &b.tail {
            out.push_str(&"  ".repeat(indent));
            out.push_str(&t.text());
            out.push('\n');
        }
    }

    fn print_stmt(&self, s: &S, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        out.push_str(&pad);
        match s {
            S::Bind(id, ty, init) => {
                let b = &self.binds[*id];
                let kw = if b.kind == Kind::Var && !b.assigned {
                    "val"
                } else if matches!(b.kind, Kind::Var | Kind::Counter) {
                    "var"
                } else {
                    "val"
                };
                match ty {
                    Some(ty) => {
                        out.push_str(&format!("{} {}: {} = {}\n", kw, b.name, ty, init.text()))
                    }
                    None => out.push_str(&format!("{} {} = {}\n", kw, b.name, init.text())),
                }
            }
            S::Assign(p, op, v) => out.push_str(&format!("{} {} {}\n", p.text(), op, v.text())),
            S::Discard(e) => out.push_str(&format!("_ = {}\n", e.text())),
            S::Expr(e) => out.push_str(&format!("{}\n", e.text())),
            S::If(c, then, els) => {
                out.push_str(&format!("if {} {{\n", c.header()));
                self.print_block(then, indent + 1, out);
                let mut els = els.as_ref();
                while let Some(e) = els {
                    // An `else` block that holds one `if` alone is written `else if`.
                    if let (None, [S::If(c, then, next)]) = (&e.tail, e.stmts.as_slice()) {
                        out.push_str(&format!("{}}} else if {} {{\n", pad, c.header()));
                        self.print_block(then, indent + 1, out);
                        els = next.as_ref();
                    } else {
                        out.push_str(&format!("{}}} else {{\n", pad));
                        self.print_block(e, indent + 1, out);
                        els = None;
                    }
                }
                out.push_str(&format!("{}}}\n", pad));
            }
            S::Match(s, arms) => {
                out.push_str(&format!("match {} {{\n", s.header()));
                for (p, body) in arms {
                    out.push_str(&format!("{}  {} => {{\n", pad, p));
                    self.print_block(body, indent + 2, out);
                    out.push_str(&format!("{}  }}\n", pad));
                }
                out.push_str(&format!("{}}}\n", pad));
            }
            S::While(c, clauses, body) => {
                out.push_str(&format!("while {}", c.header()));
                for k in clauses {
                    out.push(' ');
                    out.push_str(k);
                }
                out.push_str(" {\n");
                self.print_block(body, indent + 1, out);
                out.push_str(&format!("{}}}\n", pad));
            }
            S::Guard(c, els) => {
                out.push_str(&format!("guard {} else {{\n", c.header()));
                self.print_block(els, indent + 1, out);
                out.push_str(&format!("{}}}\n", pad));
            }
            S::Return(Some(e)) => out.push_str(&format!("return {}\n", e.text())),
            S::Return(None) => out.push_str("return\n"),
            S::Break => out.push_str("break\n"),
            S::Continue => out.push_str("continue\n"),
        }
    }
}

/// A field as its declaration writes it: `name: type`, with its refinement when it has one.
fn field_text(f: &GField) -> String {
    let range = |r: R| match (f.exclusive, r.hi.add(Num::small(1))) {
        (true, Some(end)) => format!("[{}..{}]", r.lo, end),
        _ => format!("[{}..={}]", r.lo, r.hi),
    };
    match (&f.ty, f.refine) {
        (Ty::Int(t), Some(r)) => format!("{}: {}{}", f.name, t.name, range(r)),
        (Ty::Array(n, e), Some(r)) => format!("{}: [{}]{}{}", f.name, n, e, range(r)),
        (ty, _) => format!("{}: {}", f.name, ty),
    }
}

fn compound(op: Op) -> &'static str {
    match op {
        Op::Add => "+=",
        Op::Sub => "-=",
        Op::Mul => "*=",
        Op::Div => "/=",
        Op::Rem => "%=",
        Op::BitAnd => "&=",
        Op::BitOr => "|=",
        Op::BitXor => "^=",
        Op::Shl => "<<=",
        _ => ">>=",
    }
}

fn req_text(r: &Req, params: &[Param]) -> String {
    let n = |i: usize| params[i].name.clone();
    match *r {
        Req::Bound(a, op, c) => format!("{} {} {}", n(a), op.text(), c),
        Req::Order(a, op, b) => format!("{} {} {}", n(a), op.text(), n(b)),
        Req::Sum(a, b, c) => format!("{} + {} <= {}", n(a), n(b), c),
        Req::Below(a, b) => format!("{} < {}.len", n(a), n(b)),
    }
}

// -------------------------------------------------------------------- programs that check

/// A generated program that passes every check, Tier 1 included, so that the three paths run
/// it: the program of its seed, with each function Tier 1 refused generated again.
pub struct Checked {
    pub program: Program,
    /// The checked module; none when `problems` tells why the program does not check.
    pub module: Option<CheckedModule>,
    /// Each narrowing Tier 1 proved always traps (`E0508`), whose function was generated again,
    /// with what the reference interpreter saw of the claim.
    pub regenerated: Vec<AlwaysTraps>,
    /// What no generated program may show: a diagnostic other than `E0508`, which is a bug of
    /// the generator; an `E0508` the reference interpreter contradicts, which is a bug of
    /// Tier 1; and a program that still has one after [`MAX_ROUNDS`].
    pub problems: Vec<String>,
}

/// A narrowing Tier 1 proved always traps where it is reached (`E0508`), in a generated
/// function, and the runs of that function that checked it.
#[derive(Clone, Debug)]
pub struct AlwaysTraps {
    pub function: String,
    pub line: u32,
    pub col: u32,
    /// The claim, as Tier 1 words it.
    pub message: String,
    /// Whether the claim is about the value of the expression it points at, which the runs
    /// can see; a compound assignment's is about `x + e` and points at `e`.
    pub seen: bool,
    /// The calls made of the function.
    pub calls: usize,
    /// Those that gave the expression a value: every time outside its type, as claimed.
    pub reached: usize,
}

/// The least magnitude of an operand [`Gen::near_edge`] reads as lying near 2^62 or beyond.
const EDGE_OPERAND: u128 = 1 << 61;

/// The most bindings a function that must prove a relation between two values declares
/// ([`Gen::relates`]): with its calls' arguments and its fields they stay well within the
/// variables Tier 1 relates.
const RELATED: usize = 30;

/// How many times a program's functions are generated again before it is given up.
const MAX_ROUNDS: usize = 12;

/// The calls the reference interpreter makes of a function to check a claim about it.
const CLAIM_CALLS: CallPlan = CallPlan {
    random: 24,
    exhaustive: 256,
};

/// The program of `seed` that passes every check. A function with a narrowing Tier 1 proves
/// always traps is refused (`E0508`): the reference interpreter first runs the function on
/// generated arguments, and every value the narrowed expression takes must lie outside the type
/// as Tier 1 claims, or the claim is reported as a problem; then the function's body is
/// generated again, under the same signature. Nothing else may refuse a generated program.
pub fn checked(seed: u64) -> Checked {
    let mut attempts: Vec<u32> = Vec::new();
    let mut regenerated = Vec::new();
    let mut problems = Vec::new();
    for round in 0.. {
        let program = generated(seed, &attempts);
        let refused = |program, regenerated, problems| Checked {
            program,
            module: None,
            regenerated,
            problems,
        };
        let diagnostics = match panic::catch_unwind(|| mente_sema::check_program(&program.text)) {
            Ok(Ok(module)) => {
                return Checked {
                    program,
                    module: Some(module),
                    regenerated,
                    problems,
                }
            }
            Ok(Err(diagnostics)) => diagnostics,
            Err(_) => {
                problems.push(format!("seed {}: the checker panicked", seed));
                return refused(program, regenerated, problems);
            }
        };
        let others: Vec<String> = diagnostics
            .iter()
            .filter(|d| d.code != "E0508")
            .map(|d| {
                format!(
                    "seed {}: {} at {}:{}: {}",
                    seed, d.code, d.primary.line, d.primary.col, d.message
                )
            })
            .collect();
        if !others.is_empty() {
            problems.extend(others);
            return refused(program, regenerated, problems);
        }
        if round == MAX_ROUNDS {
            problems.push(format!(
                "seed {}: a narrowing still always traps after {} rounds of generating its function again",
                seed, MAX_ROUNDS
            ));
            return refused(program, regenerated, problems);
        }
        // Tier 1 alone refuses the program, so without it the functions run.
        let Ok(Ok(kept)) =
            panic::catch_unwind(|| mente_sema::check_program_without_proofs(&program.text))
        else {
            problems.push(format!(
                "seed {}: Tier 1 alone refuses the program, which does not check without it",
                seed
            ));
            return refused(program, regenerated, problems);
        };
        attempts.resize(program.functions.len(), 0);
        let mut again = BTreeSet::new();
        for d in &diagnostics {
            let Some(k) = program
                .functions
                .iter()
                .position(|(_, at)| at.contains(&d.primary.start))
            else {
                problems.push(format!(
                    "seed {}: E0508 at {}:{}, in no function",
                    seed, d.primary.line, d.primary.col
                ));
                return refused(program, regenerated, problems);
            };
            regenerated.push(always_traps(&kept, &program, k, d, &mut problems));
            again.insert(k);
        }
        for k in again {
            attempts[k] += 1;
        }
    }
    unreachable!("the rounds end at MAX_ROUNDS")
}

/// What an `E0508` claims of a value: that it is at most, or at least, `bound`, which lies
/// outside the integer type `ty` the value is narrowed into.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Claim {
    ty: mente_sema::types::IntTy,
    at_most: bool,
    bound: i128,
    /// Whether the value is that of the expression the diagnostic points at.
    seen: bool,
}

impl Claim {
    /// The claim of an `E0508`'s `message`, whose span holds `source`: none for a message of
    /// another form, which is then Tier 1's to keep in step with this reading.
    fn read(message: &str, source: &str) -> Option<Claim> {
        let (before, after) = message.split_once(" here, outside `")?;
        let (ty, _) = after.split_once('`')?;
        let ty = *mente_sema::types::INT_TYPES.iter().find(|t| t.name == ty)?;
        let (before, bound) = before.rsplit_once(' ')?;
        let bound: i128 = bound.parse().ok()?;
        let (what, at_most) = match before.strip_suffix(" is at most") {
            Some(what) => (what, true),
            None => (before.strip_suffix(" is at least")?, false),
        };
        let what = what.strip_prefix("Tier 1 proves ")?;
        let seen = what == "the value"
            || what
                .strip_prefix('`')
                .and_then(|w| w.strip_suffix('`'))
                .is_some_and(|w| w == source);
        Some(Claim {
            ty,
            at_most,
            bound,
            seen,
        })
    }

    /// Why `x`, a value the expression took, contradicts the claim, if it does.
    fn broken_by(&self, x: Int) -> Option<String> {
        let bound = Int::from_i128(self.bound);
        if x.fits(self.ty) {
            Some(format!(
                "{} fits `{}`, so the narrowing does not trap",
                x, self.ty.name
            ))
        } else if self.at_most && x > bound {
            Some(format!("{} is above {}", x, bound))
        } else if !self.at_most && x < bound {
            Some(format!("{} is below {}", x, bound))
        } else {
            None
        }
    }
}

/// Checks the `E0508` `d` of function `k` of `program` by running the function: the reference
/// interpreter calls it on generated arguments in `kept`, the module checked without Tier 1,
/// and every integer the expression `d` points at gives, an array's elements included, must be
/// what Tier 1 claims. A value that is not is pushed on `problems` with its call.
fn always_traps(
    kept: &CheckedModule,
    program: &Program,
    k: usize,
    d: &Diagnostic,
    problems: &mut Vec<String>,
) -> AlwaysTraps {
    let name = &program.functions[k].0;
    let at = d.primary;
    let mut out = AlwaysTraps {
        function: name.clone(),
        line: at.line,
        col: at.col,
        message: d.message.clone(),
        seen: false,
        calls: 0,
        reached: 0,
    };
    let source = program.text.get(at.start..at.end).unwrap_or("");
    let Some(claim) = Claim::read(&d.message, source) else {
        problems.push(format!(
            "seed {}: an E0508 whose claim is not read: {}",
            program.seed, d.message
        ));
        return out;
    };
    out.seen = claim.seen;
    if !claim.seen {
        return out;
    }
    let limits = mente_interp::Limits {
        max_depth: 64,
        max_steps: 200_000,
        ..mente_interp::Limits::default()
    };
    let mut rng = Rng::new(program.seed ^ 0x0E05_08C1_A135_7EED);
    let calls = arguments_of(kept, name, &mut rng, CLAIM_CALLS, limits.max_array);
    for args in calls.unwrap_or_default() {
        out.calls += 1;
        let text: Vec<String> = args
            .iter()
            .map(|(is_mut, v)| format!("{}{}", if *is_mut { "mut " } else { "" }, v))
            .collect();
        let values = args.into_iter().map(|(_, v)| v).collect();
        let mut taken: Vec<Int> = Vec::new();
        // The run's end, a value or a trap, says nothing of the claim: what the expression
        // gave before it does.
        let _ =
            mente_interp::call_observed(kept, name, values, limits, &mut |span: Span, x: Int| {
                if (span.start, span.end) == (at.start, at.end) {
                    taken.push(x);
                }
            });
        if !taken.is_empty() {
            out.reached += 1;
        }
        if let Some(why) = taken.iter().find_map(|x| claim.broken_by(*x)) {
            problems.push(format!(
                "seed {}: E0508 at {}:{} is wrong: {}\n  in the call {}({}): {}",
                program.seed,
                at.line,
                at.col,
                d.message,
                name,
                text.join(", "),
                why
            ));
            break;
        }
    }
    out
}

// -------------------------------------------------------------------- arguments

/// The values a parameter takes from outside the program.
#[derive(Clone, Debug)]
enum Domain {
    Int(IntT, Option<(Bound, Bound, bool)>),
    Bool,
    Array(u32, Box<Domain>),
    /// A struct by its name, and each field's name and values in the order of its declaration.
    Struct(String, Vec<(String, Domain)>),
    /// An enum by its name, and each variant's name with its payload's fields' names and
    /// values in the order of the declaration.
    Enum(String, Vec<(String, Vec<(String, Domain)>)>),
}

impl Domain {
    /// The values an argument of this domain is built of, itself, each element and each field
    /// counted one each: `[4294967295][0]u8` holds 4,294,967,296, though none is a scalar.
    fn values(&self) -> u128 {
        match self {
            Domain::Int(..) | Domain::Bool => 1,
            Domain::Array(n, elem) => u128::from(*n)
                .saturating_mul(elem.values())
                .saturating_add(1),
            Domain::Struct(_, fields) => fields
                .iter()
                .fold(1, |sum, (_, f)| sum.saturating_add(f.values())),
            // The variant with the most values, its tag counted.
            Domain::Enum(_, variants) => variants
                .iter()
                .map(|(_, fields)| {
                    fields
                        .iter()
                        .fold(1, |sum: u128, (_, f)| sum.saturating_add(f.values()))
                })
                .max()
                .unwrap_or(1),
        }
    }
}

fn domain(tree: &Tree, t: TypeId, structs: &Structs) -> Option<Domain> {
    Some(match &tree.ty(t).kind {
        TypeKind::Int { name, refinement } => Domain::Int(
            IntT::named(name)?,
            refinement
                .as_ref()
                .map(|r| (r.lo.clone(), r.hi.clone(), r.inclusive)),
        ),
        TypeKind::Bool => Domain::Bool,
        TypeKind::Array { len, elem } => Domain::Array(
            u32::try_from(*len).ok()?,
            Box::new(domain(tree, *elem, structs)?),
        ),
        TypeKind::Named(name) => {
            let def = structs.get(name)?;
            let fields = def
                .fields
                .iter()
                .map(|f| Some((f.name.clone(), domain(def.tree(), f.decl, structs)?)))
                .collect::<Option<Vec<_>>>()?;
            Domain::Struct(name.clone(), fields)
        }
        TypeKind::Enum(name) => {
            let def = structs.get_enum(name)?;
            let variants = def
                .variants
                .iter()
                .map(|v| {
                    let fields = v
                        .fields
                        .iter()
                        .map(|f| Some((f.name.clone(), domain(def.tree(), f.decl, structs)?)))
                        .collect::<Option<Vec<_>>>()?;
                    Some((v.name.clone(), fields))
                })
                .collect::<Option<Vec<_>>>()?;
            Domain::Enum(name.clone(), variants)
        }
        TypeKind::Error => return None,
    })
}

/// How calls are chosen: `random` calls of each public function, unless the product of its
/// parameters' value counts is at most `exhaustive`, when every combination is called.
#[derive(Clone, Copy, Debug)]
pub struct CallPlan {
    pub random: usize,
    pub exhaustive: u128,
}

/// The calls of every public function of `module`, whose text is `src`: arguments that fit
/// their parameters' types and refinements, every combination where few exist, and otherwise
/// random ones, the types' extremes first.
pub fn calls(src: &str, module: &CheckedModule, seed: u64, plan: CallPlan) -> Vec<CallArgs> {
    // The reference interpreter cannot build a literal of more elements.
    let most = mente_interp::Limits::default().max_array;
    calls_within(src, module, seed, plan, most)
}

/// [`calls`], leaving out a function whose arguments hold more than `most` values, each
/// element and field counted.
pub fn calls_within(
    src: &str,
    module: &CheckedModule,
    seed: u64,
    plan: CallPlan,
    most: u128,
) -> Vec<CallArgs> {
    let Ok(decls) = mente_syntax::parse_module(src) else {
        return Vec::new();
    };
    let public: Vec<String> = decls
        .items
        .iter()
        .filter_map(|i| match i {
            mente_syntax::ItemDecl::Fn(f) if f.is_pub => Some(f.name.clone()),
            _ => None,
        })
        .collect();
    let mut rng = Rng::new(seed ^ 0xA5A5_5A5A_C3C3_3C3C);
    let mut out = Vec::new();
    for name in &module.names {
        if !public.contains(name) {
            continue;
        }
        for args in arguments_of(module, name, &mut rng, plan, most).unwrap_or_default() {
            out.push((name.clone(), args));
        }
    }
    out
}

/// The argument lists `plan` gives function `name` of `module`; none for a function without a
/// body, one with a parameter whose type the generator has no values of, or one whose arguments
/// hold more than `most` values, which the reference interpreter could not hold.
fn arguments_of(
    module: &CheckedModule,
    name: &str,
    rng: &mut Rng,
    plan: CallPlan,
    most: u128,
) -> Option<Vec<Vec<(bool, Value)>>> {
    let f = module.functions.get(module.index(name)?)?.as_ref()?;
    let tree = &f.syntax.tree;
    let params: Vec<(String, bool, Domain)> = f
        .syntax
        .params
        .iter()
        .map(|p| {
            Some((
                p.name.clone(),
                p.is_mut,
                domain(tree, p.ty, &module.structs)?,
            ))
        })
        .collect::<Option<_>>()?;
    if params.iter().any(|(_, _, d)| d.values() > most) {
        return None;
    }
    Some(function_calls(&params, rng, plan))
}

/// The argument lists of one function, each argument marked `mut` as its parameter is.
fn function_calls(
    params: &[(String, bool, Domain)],
    rng: &mut Rng,
    plan: CallPlan,
) -> Vec<Vec<(bool, Value)>> {
    let mark = |values: Vec<Value>| -> Vec<(bool, Value)> {
        params.iter().map(|p| p.1).zip(values).collect()
    };
    if let Some(all) = every(params, plan.exhaustive) {
        return all.into_iter().map(mark).collect();
    }
    let mut out = Vec::new();
    for k in 0..plan.random {
        // The first call takes every lowest value, the second every highest, then each value
        // is an extreme now and then and random otherwise.
        let how = match k {
            0 => Pick::Low,
            1 => Pick::High,
            _ => Pick::Mixed,
        };
        if let Some(values) = one_call(params, rng, how) {
            out.push(mark(values));
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Low,
    High,
    Mixed,
}

/// Every combination of values, when each parameter is an integer of literal bounds or a
/// `bool` and they are at most `limit` together.
fn every(params: &[(String, bool, Domain)], limit: u128) -> Option<Vec<Vec<Value>>> {
    let mut ranges: Vec<Vec<Value>> = Vec::new();
    let mut total: u128 = 1;
    for (_, _, d) in params {
        let values = match d {
            Domain::Bool => vec![Value::Bool(false), Value::Bool(true)],
            Domain::Int(t, r) => {
                let (lo, hi) = literal_bounds(*t, r.as_ref())?;
                let count = hi.sub(lo)?.mag.checked_add(1)?;
                if count > limit {
                    return None;
                }
                let mut v = Vec::new();
                let mut n = lo;
                while n <= hi {
                    v.push(n.value());
                    n = n.add(Num::small(1))?;
                }
                v
            }
            // An enum whose variants hold nothing takes each variant.
            Domain::Enum(name, variants) if variants.iter().all(|(_, f)| f.is_empty()) => variants
                .iter()
                .map(|(v, _)| Value::Enum(name.clone(), v.clone(), Vec::new()))
                .collect(),
            Domain::Array(..) | Domain::Struct(..) | Domain::Enum(..) => return None,
        };
        total = total.checked_mul(values.len() as u128)?;
        if total > limit {
            return None;
        }
        ranges.push(values);
    }
    let mut out: Vec<Vec<Value>> = vec![Vec::new()];
    for values in ranges {
        out = out
            .into_iter()
            .flat_map(|prefix| {
                values.iter().map(move |v| {
                    let mut p = prefix.clone();
                    p.push(v.clone());
                    p
                })
            })
            .collect();
    }
    Some(out)
}

fn bound_num(b: &Bound) -> Option<Num> {
    match b {
        Bound::Int { negative, value } => Some(Num::new(*negative, *value)),
        _ => None,
    }
}

/// The lowest and highest value of an integer of type `t` with refinement `r`, when its bounds
/// are literals.
fn literal_bounds(t: IntT, r: Option<&(Bound, Bound, bool)>) -> Option<(Num, Num)> {
    let Some((lo, hi, inclusive)) = r else {
        return Some((t.min(), t.max()));
    };
    let lo = bound_num(lo)?.max(t.min());
    let hi = bound_num(hi)?;
    let hi = if *inclusive {
        hi
    } else {
        hi.sub(Num::small(1))?
    }
    .min(t.max());
    Some((lo, hi))
}

/// One argument list, or `None` when a refinement leaves a parameter no value.
fn one_call(params: &[(String, bool, Domain)], rng: &mut Rng, how: Pick) -> Option<Vec<Value>> {
    let mut values: Vec<Option<Value>> = vec![None; params.len()];
    // A bound may name a parameter declared later: values are chosen once those it names are.
    for _ in 0..=params.len() {
        for k in 0..params.len() {
            if values[k].is_some() {
                continue;
            }
            if let Some(v) = pick(&params[k].2, params, &values, rng, how) {
                values[k] = Some(v?);
            }
        }
    }
    values.into_iter().collect()
}

/// A value of domain `d`: `None` while a bound names a parameter without a value yet, and
/// `Some(None)` when its range is empty.
fn pick(
    d: &Domain,
    params: &[(String, bool, Domain)],
    values: &[Option<Value>],
    rng: &mut Rng,
    how: Pick,
) -> Option<Option<Value>> {
    match d {
        Domain::Bool => Some(Some(Value::Bool(match how {
            Pick::Low => false,
            Pick::High => true,
            Pick::Mixed => rng.chance(50),
        }))),
        Domain::Array(n, e) => {
            let mut items = Vec::new();
            for _ in 0..*n {
                match pick(e, params, values, rng, how)? {
                    Some(v) => items.push(v),
                    None => return Some(None),
                }
            }
            Some(Some(Value::Array(items)))
        }
        // A field's refinement has literal bounds, which name no parameter.
        Domain::Struct(name, fields) => {
            let mut out = Vec::new();
            for (f, d) in fields {
                match pick(d, params, values, rng, how)? {
                    Some(v) => out.push((f.clone(), v)),
                    None => return Some(None),
                }
            }
            Some(Some(Value::Struct(name.clone(), out)))
        }
        // The first variant at the lowest, the last at the highest, any one at random; then its
        // payload, as a struct's fields.
        Domain::Enum(name, variants) => {
            let k = match how {
                Pick::Low => 0,
                Pick::High => variants.len().saturating_sub(1),
                Pick::Mixed => rng.below(variants.len().max(1) as u64) as usize,
            };
            let (v, fields) = variants.get(k)?;
            let mut out = Vec::new();
            for (f, d) in fields {
                match pick(d, params, values, rng, how)? {
                    Some(x) => out.push((f.clone(), x)),
                    None => return Some(None),
                }
            }
            Some(Some(Value::Enum(name.clone(), v.clone(), out)))
        }
        Domain::Int(t, r) => {
            let (lo, hi) = match r {
                None => (t.min(), t.max()),
                Some((lo, hi, inclusive)) => {
                    let lo = resolve(lo, params, values)?;
                    let hi = resolve(hi, params, values)?;
                    let hi = if *inclusive {
                        Some(hi)
                    } else {
                        hi.sub(Num::small(1))
                    };
                    match hi {
                        Some(hi) => (lo.max(t.min()), hi.min(t.max())),
                        None => return Some(None),
                    }
                }
            };
            if lo > hi {
                return Some(None);
            }
            let range = R { lo, hi };
            let n = match how {
                Pick::Low => lo,
                Pick::High => hi,
                Pick::Mixed => match rng.below(10) {
                    0 => lo,
                    1 => hi,
                    2 => lo.add(Num::small(1)).unwrap_or(lo).min(hi),
                    3 => hi.sub(Num::small(1)).unwrap_or(hi).max(lo),
                    4..=5 => Num::small(rng.below(17) as i64 - 8).clamp(lo, hi),
                    _ => {
                        let span = hi.sub(lo).map_or(u128::MAX, |s| s.mag);
                        let off = match span.checked_add(1) {
                            Some(m) => rng.u128() % m,
                            None => rng.u128(),
                        };
                        lo.add(Num::new(false, off)).unwrap_or(lo)
                    }
                },
            };
            debug_assert!(range.holds(n));
            Some(Some(n.value()))
        }
    }
}

/// A refinement bound's value: a literal, another parameter's value, or an array's length.
fn resolve(b: &Bound, params: &[(String, bool, Domain)], values: &[Option<Value>]) -> Option<Num> {
    match b {
        Bound::Int { negative, value } => Some(Num::new(*negative, *value)),
        Bound::Name(n) => {
            let k = params.iter().position(|p| &p.0 == n)?;
            match values[k].as_ref()? {
                Value::Int(i) => Some(Num::new(i.is_negative(), i.magnitude())),
                _ => None,
            }
        }
        Bound::Len(n) => {
            let k = params.iter().position(|p| &p.0 == n)?;
            match &params[k].2 {
                Domain::Array(len, _) => Some(Num::new(false, u128::from(*len))),
                _ => None,
            }
        }
    }
}

/// Whether the pattern text `p` binds no name: every field it names is bound to `_`.
fn binds_nothing(p: &str) -> bool {
    p.split(": ").skip(1).all(|rest| rest.starts_with('_'))
}

// -------------------------------------------------------------------- tests of the claim check

mod tests {
    use super::*;

    /// A module of one function `f`, as a generated program.
    fn written(body: &str) -> Program {
        let text = format!("module m\n{}\n", body);
        Program {
            seed: 0,
            mode: Mode::Plain,
            functions: vec![("f".to_string(), 0..text.len())],
            text,
        }
    }

    /// The `E0508`s of `program`, each with what the reference interpreter saw of it, and the
    /// problems found.
    fn claims(program: &Program) -> (Vec<AlwaysTraps>, Vec<String>) {
        let diagnostics = mente_sema::check_program(&program.text)
            .err()
            .unwrap_or_default();
        let kept = mente_sema::check_program_without_proofs(&program.text).unwrap();
        let mut problems = Vec::new();
        let seen = diagnostics
            .iter()
            .filter(|d| d.code == "E0508")
            .map(|d| always_traps(&kept, program, 0, d, &mut problems))
            .collect();
        (seen, problems)
    }

    /// A `match` covers the refinement of what it reads (`core-calculus.md`, item 14), so the
    /// generator knows it wherever the checker does: of a binding, of a field, of an element
    /// of an array field, and of an element of what a pattern bound to such a field, whose
    /// own range, as `E0222` takes it, stays its type's.
    #[test]
    fn test_a_pattern_binding_keeps_the_refinement_a_match_covers() {
        let r = R {
            lo: Num::small(2),
            hi: Num::small(16),
        };
        let field = |name: &str, ty: Ty| GField {
            name: name.into(),
            ty,
            refine: Some(r),
            exclusive: false,
            is_pub: true,
        };
        let int = Ty::Int(INTS[7]);
        let array = Ty::Array(1, Box::new(int.clone()));
        let mut g = Gen::new(0, Mode::Refined);
        g.scopes = vec![Vec::new()];
        let pattern = Pat::Variant(
            "E".into(),
            "V".into(),
            vec![(field("y0", int), true), (field("y1", array.clone()), true)],
        );
        let ids = g.bind_pattern(&pattern);
        let (Some(scalar), Some(elements)) = (ids[0], ids[1]) else {
            panic!("the pattern binds both fields");
        };
        // An array no pattern bound has elements of its type's range.
        let plain = g.declare("v".into(), array, Kind::Val, None);
        let name = |id: usize| E::name(&g.binds[id].name);
        let first = |e: E| E::Index(Box::new(e), Box::new(E::Lit("0".into())));
        assert_eq!(g.declared_range(&name(scalar)), Some(r));
        assert_eq!(g.declared_range(&first(name(elements))), Some(r));
        assert_eq!(g.declared_range(&name(elements)), None);
        assert_eq!(g.declared_range(&first(name(plain))), None);
    }

    /// What a call passes to parameters its callee relates, by a `requires` of two of them or
    /// a bound that names one: in a function that may grow past the variables Tier 1 relates,
    /// at most one of them names a binding, since bounds alone prove a name against a literal
    /// and nothing of two names; in one written to stay small both may, but never one name
    /// on both sides of a sum, which Tier 1 holds in no function.
    #[test]
    fn test_a_call_relates_two_names_only_where_tier1_relates_them() {
        let param = |name: &str, refine: Option<Refine>| Param {
            name: name.into(),
            is_mut: false,
            ty: Ty::Int(INTS[2]),
            refine,
        };
        // A parameter of a 128-bit type, whose literals reach its limits.
        let wide = |name: &str| Param {
            name: name.into(),
            is_mut: false,
            ty: Ty::Int(INTS[10]),
            refine: None,
        };
        let below = |i: usize| {
            Some(Refine {
                lo: Bnd::Lit(Num::ZERO),
                hi: Bnd::Name(i),
                inclusive: false,
            })
        };
        let sigs = [
            (
                vec![param("a", None), param("b", below(0))],
                Vec::new(),
                false,
            ),
            (
                vec![param("a", None), param("b", None)],
                vec![Req::Order(0, Op::Lt, 1)],
                false,
            ),
            (
                vec![param("a", None), wide("b")],
                vec![Req::Sum(0, 1, Num::small(9))],
                true,
            ),
        ];
        let named = |e: &E| {
            let mut out = Vec::new();
            for n in ["x", "y", "z"] {
                if names(e, &|m| m == n) {
                    out.push(n);
                }
            }
            out
        };
        for (params, requires, sum) in sigs {
            let sig = Sig {
                name: "callee".into(),
                params,
                ret: None,
                ret_range: None,
                requires,
                cost: 0,
            };
            for small in [false, true] {
                let (mut both, mut one, mut none) = (0, 0, 0);
                for seed in 0..400 {
                    let mut g = Gen::new(seed, Mode::Refined);
                    g.scopes = vec![Vec::new()];
                    g.small = small;
                    for n in ["x", "y", "z"] {
                        g.declare(n.into(), Ty::Int(INTS[2]), Kind::Val, None);
                    }
                    let Some(E::Call(_, args)) = g.arguments(&sig, false, 0) else {
                        panic!("no call");
                    };
                    let (a, b) = (named(&args[0].1), named(&args[1].1));
                    match (a.is_empty(), b.is_empty()) {
                        (false, false) => both += 1,
                        (true, true) => none += 1,
                        _ => one += 1,
                    }
                    assert!(
                        small || a.is_empty() || b.is_empty(),
                        "two names, {:?} and {:?}, in a function that may grow",
                        a,
                        b
                    );
                    assert!(
                        !sum || a.iter().all(|n| !b.contains(n)),
                        "{:?} on both sides of a sum",
                        a
                    );
                    // A literal a sum takes is one the clause may allow.
                    for (_, e) in &args {
                        if let (true, Some(n)) = (sum, constant(e)) {
                            assert!(Num::ZERO <= n && n <= Num::small(9), "{} in a sum", n);
                        }
                    }
                    // Each name the call reads is read, and none it gave up is.
                    for (id, n) in ["x", "y", "z"].iter().enumerate() {
                        assert_eq!(
                            g.binds[id].read,
                            a.contains(n) || b.contains(n),
                            "{} in {:?}, {:?}",
                            n,
                            a,
                            b
                        );
                    }
                }
                // Where it may, a call relates two names most of the time; where it may not,
                // one name and a literal, and two literals now and then.
                assert!(
                    small || (one > 40 && none > 0),
                    "{} with one name, {} with none",
                    one,
                    none
                );
                assert_eq!(both > 200, small, "{} with two names", both);
            }
        }
    }

    /// The class the generator gives an expression is the type the checker gives it, for
    /// every pair of classes: one type, literals alone, and several types with the type that
    /// holds them; and an operand the checker types in 128 bits, whose operations it checks at
    /// the operator, is one the generator reads as wide.
    #[test]
    fn test_the_class_of_an_expression_is_the_checkers_type_of_it() {
        use mente_sema::types::{Int as Checked, INT_TYPES};
        let checked = |c: Cls| match c {
            Cls::Lit => Checked::Literal,
            Cls::One(t) => Checked::One(*INT_TYPES.iter().find(|u| u.name == t.name).unwrap()),
            Cls::Mixed(t) => {
                Checked::Mixed(t.map(|t| *INT_TYPES.iter().find(|u| u.name == t.name).unwrap()))
            }
        };
        let mut classes: Vec<Cls> = INTS.iter().map(|t| Cls::One(*t)).collect();
        classes.push(Cls::Lit);
        // The types that hold two others, and the mix that none holds.
        for a in INTS {
            for b in INTS {
                classes.push(Cls::One(a).join(Cls::One(b)));
            }
        }
        classes.dedup();
        assert!(classes.contains(&Cls::Mixed(None)));
        let mut wide = 0;
        for &a in &classes {
            for &b in &classes {
                let joined = a.join(b);
                assert_eq!(
                    checked(joined),
                    checked(a).join(checked(b)),
                    "{:?} with {:?}",
                    a,
                    b
                );
                // An operand the checker types in 128 bits is one this reads as wide.
                let bits = match checked(joined) {
                    Checked::One(t) | Checked::Mixed(Some(t)) => t.bits,
                    Checked::Literal | Checked::Mixed(None) => 0,
                };
                assert_eq!(joined.wide().is_some(), bits == 128, "{:?} with {:?}", a, b);
                wide += usize::from(bits == 128 && matches!(joined, Cls::Mixed(_)));
            }
        }
        assert!(wide > 0, "no mix of types is held in 128 bits");
    }

    /// An `E0508`'s claim is read from the message Tier 1 writes, so that a change of its
    /// wording fails here and not by leaving the claims unchecked: the type, the side and the
    /// bound, and whether the value is the one of the expression the diagnostic points at. A
    /// compound assignment's is `x + e`, and it points at `e`.
    #[test]
    fn test_an_always_trap_claim_is_read_from_its_message() {
        let read = |body: &str| -> Vec<Claim> {
            let p = written(body);
            let diagnostics = mente_sema::check_program(&p.text).err().unwrap_or_default();
            assert!(!diagnostics.is_empty(), "{}", body);
            diagnostics
                .iter()
                .map(|d| {
                    assert_eq!(d.code, "E0508", "{}", d.message);
                    let source = &p.text[d.primary.start..d.primary.end];
                    Claim::read(&d.message, source).expect("a claim")
                })
                .collect()
        };
        let ty = |name: &str| {
            *mente_sema::types::INT_TYPES
                .iter()
                .find(|t| t.name == name)
                .unwrap()
        };
        assert_eq!(
            read("pub fn f(x: u8) -> u8 {\n  val y: i16 = 0 - x - 1\n  y\n}"),
            [Claim {
                ty: ty("u8"),
                at_most: true,
                bound: -1,
                seen: true
            }]
        );
        assert_eq!(
            read("fn g(a: u8) -> u8 = a\npub fn f(x: u8) -> u8 = g(x + 256)"),
            [Claim {
                ty: ty("u8"),
                at_most: false,
                bound: 256,
                seen: true
            }]
        );
        assert_eq!(
            read("pub fn f(x: u8) -> u8 {\n  var v: u8 = x\n  v += 300\n  v\n}"),
            [Claim {
                ty: ty("u8"),
                at_most: false,
                bound: 300,
                seen: false
            }]
        );
        assert_eq!(Claim::read("Tier 1 proves nothing here", "x"), None);
    }

    /// A value the expression took breaks a claim when it fits the type, so that the narrowing
    /// would not trap, or lies past the bound Tier 1 names on the other side of the type.
    #[test]
    fn test_a_value_inside_the_type_or_past_the_bound_breaks_a_claim() {
        let u8_ = *mente_sema::types::INT_TYPES
            .iter()
            .find(|t| t.name == "u8")
            .unwrap();
        let below = Claim {
            ty: u8_,
            at_most: true,
            bound: -1,
            seen: true,
        };
        let above = Claim {
            at_most: false,
            bound: 256,
            ..below
        };
        let broken = |c: &Claim, x: i128| c.broken_by(Int::from_i128(x));
        assert_eq!(broken(&below, -1), None);
        assert_eq!(broken(&below, -700), None);
        assert_eq!(
            broken(&below, 0).as_deref(),
            Some("0 fits `u8`, so the narrowing does not trap")
        );
        assert_eq!(broken(&below, 300).as_deref(), Some("300 is above -1"));
        assert_eq!(broken(&above, 256), None);
        assert_eq!(
            broken(&above, 255).as_deref(),
            Some("255 fits `u8`, so the narrowing does not trap")
        );
        assert_eq!(broken(&above, -3).as_deref(), Some("-3 is below 256"));
    }

    /// The check runs the function: every call reaches a narrowing at its top and sees the
    /// value outside the type; a narrowing behind a branch is reached by the calls that take
    /// the branch; and a compound assignment's claim, about a value no expression of the
    /// program gives, is not run. None of them is a problem, since each claim holds.
    #[test]
    fn test_an_always_trap_claim_is_checked_by_running_its_function() {
        let (seen, problems) = claims(&written(
            "pub fn f(x: u8) -> u8 {\n  val y: i16 = 0 - x - 1\n  y\n}",
        ));
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(seen.len(), 1);
        assert!(seen[0].seen && seen[0].calls > 0, "{:?}", seen[0]);
        assert_eq!(seen[0].reached, seen[0].calls, "{:?}", seen[0]);
        assert_eq!((seen[0].function.as_str(), seen[0].line), ("f", 4));

        let (seen, problems) = claims(&written(
            "pub fn f(x: u8, c: bool) -> u8 {\n  val y: i16 = 0 - x - 1\n  if c {\n    return y\n  }\n  x\n}",
        ));
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(seen.len(), 1);
        assert!(
            0 < seen[0].reached && seen[0].reached < seen[0].calls,
            "{:?}",
            seen[0]
        );

        let (seen, problems) = claims(&written(
            "pub fn f(x: u8) -> u8 {\n  var v: u8 = x\n  v += 300\n  v\n}",
        ));
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].seen, seen[0].calls), (false, 0));
    }
}

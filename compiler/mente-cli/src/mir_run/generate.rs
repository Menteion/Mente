//! A generator of well-typed M1 programs, deterministic from a seed, and of arguments for the
//! public functions of any checked module (`docs/design-brief.md`, "The quality system": a
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

use std::cmp::Ordering;
use std::fmt;

use mente_interp::{Int, Value};
use mente_sema::CheckedModule;
use mente_syntax::tree::{Bound, Tree, TypeId, TypeKind};

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
}

/// A type of M1.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Int(IntT),
    Bool,
    Array(u32, Box<Ty>),
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int(t) => write!(f, "{}", t.name),
            Ty::Bool => write!(f, "bool"),
            Ty::Array(n, e) => write!(f, "[{}]{}", n, e),
        }
    }
}

/// Whether a value of type `from` flows into a place of type `into`: integers into integers,
/// narrowed, and arrays of the same lengths element by element.
fn flows(from: &Ty, into: &Ty) -> bool {
    match (from, into) {
        (Ty::Int(_), Ty::Int(_)) | (Ty::Bool, Ty::Bool) => true,
        (Ty::Array(n, a), Ty::Array(m, b)) => n == m && flows(a, b),
        _ => false,
    }
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
    Call(String, Vec<(bool, E)>),
    Neg(Box<E>),
    BitNot(Box<E>),
    Not(Box<E>),
    Bin(Op, Box<E>, Box<E>),
    If(Box<E>, Box<E>, Box<E>),
    Array(Vec<E>),
    Repeat(Box<E>, u32),
    Builtin(&'static str, String),
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
            E::If(..) => 0,
            E::Bin(op, ..) => op.level(),
            E::Not(_) => NOT,
            E::Neg(_) | E::BitNot(_) => UNARY,
            E::Index(..) | E::Len(_) => POSTFIX,
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
        }
    }

    fn text(&self) -> String {
        let mut s = String::new();
        self.print(0, &mut s);
        s
    }
}

/// What the operands of an integer expression have in common, which decides its type
/// (`core-calculus.md` item 5): one integer type, literals alone, or several types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cls {
    One(IntT),
    Lit,
    Mixed,
}

impl Cls {
    fn join(self, o: Cls) -> Cls {
        match (self, o) {
            (Cls::One(a), Cls::One(b)) if a == b => Cls::One(a),
            (Cls::One(a), Cls::Lit) | (Cls::Lit, Cls::One(a)) => Cls::One(a),
            (Cls::Lit, Cls::Lit) => Cls::Lit,
            _ => Cls::Mixed,
        }
    }

    /// The 128-bit type that makes an operation on this operand checked at the operator.
    fn wide(self) -> Option<IntT> {
        match self {
            Cls::One(t) if t.bits == 128 => Some(t),
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
}

/// The program `seed` generates, the same on every platform.
pub fn program(seed: u64) -> Program {
    let mode = Mode::of(seed);
    let mut g = Gen::new(seed, mode);
    Program {
        seed,
        mode,
        text: g.module(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Val,
    Var,
    Param,
    MutParam,
    /// A loop's counter, which only the loop's first statement changes.
    Counter,
}

#[derive(Clone, Debug)]
struct Binding {
    name: String,
    ty: Ty,
    kind: Kind,
    /// A refinement with literal bounds, which is the range Tier 1 and `E0222` take for it.
    range: Option<R>,
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
    Bind(usize, String, E),
    Assign(E, &'static str, E),
    Discard(E),
    Expr(E),
    If(E, Block, Option<Block>),
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
    sigs: Vec<Sig>,
    /// The function being generated, which may call itself once when it has a measure.
    current: Option<Sig>,
    /// The depth bound of the function's recursion, when it recurses.
    recursion: Option<u64>,
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
            mode,
            sigs: Vec::new(),
            current: None,
            recursion: None,
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

    fn module(&mut self) -> String {
        let count = 1 + self.rng.below(5) as usize;
        let mut out = String::from("module generated\n");
        for k in 0..count {
            let public = k + 1 == count || self.rng.chance(50);
            let text = self.function(k, public);
            out.push('\n');
            out.push_str(&text);
        }
        out
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
                Ty::Array(n, Box::new(self.ty(depth + 1)))
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
        let cls = a.cls.join(b.cls);
        let wide = a.cls.wide().or(b.cls.wide());
        let r = match wide {
            Some(t) => t.range(),
            None => arithmetic(op, a.r, b.r).filter(|r| r.fits())?,
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

    /// A divisor: a literal zero becomes one most of the time, and a name is guarded positive
    /// in the modes with guards.
    fn divisor(&mut self, b: G) -> G {
        if b.r == R::point(Num::ZERO) && self.rng.chance(90) {
            return G {
                e: E::Lit("1".into()),
                cls: Cls::Lit,
                r: R::point(Num::small(1)),
                k: Some(Num::small(1)),
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
        if !r.fits() {
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
                return G {
                    e,
                    cls: Cls::One(t),
                    r,
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
                let base = E::name(&self.binds[id].name);
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

    /// An element of an array binding, down to an integer of the type `want` asks for.
    fn element(&mut self, want: Want) -> Option<G> {
        let ids: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| match leaf(&self.binds[id].ty) {
                (Ty::Int(t), true) => !matches!(want, Want::Exact(w) if w != *t),
                _ => false,
            })
            .collect();
        if ids.is_empty() {
            return None;
        }
        let id = *self.rng.pick(&ids);
        let mut e = self.read(id);
        let mut ty = self.binds[id].ty.clone();
        while let Ty::Array(n, inner) = ty {
            let i = self.index(n);
            e = E::Index(Box::new(e), Box::new(i));
            ty = *inner;
        }
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
            80..=95 => match self.int_name(None, true) {
                Some((e, ..)) => E::bin(Op::Rem, e, E::Lit(n.to_string())),
                None => E::Lit((n - 1).to_string()),
            },
            _ => E::Lit(n.to_string()),
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
        let ids: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| matches!(leaf(&self.binds[id].ty), (Ty::Bool, true)))
            .collect();
        if !ids.is_empty() && self.rng.chance(75) {
            let id = *self.rng.pick(&ids);
            let mut e = self.read(id);
            let mut ty = self.binds[id].ty.clone();
            while let Ty::Array(n, inner) = ty {
                let i = self.index(n);
                e = E::Index(Box::new(e), Box::new(i));
                ty = *inner;
            }
            return e;
        }
        E::Lit(if self.rng.chance(50) { "true" } else { "false" }.into())
    }

    /// A value of type `ty`, for a place of that type.
    fn value(&mut self, ty: &Ty, depth: u32) -> E {
        match ty {
            Ty::Int(t) => self.narrowed(*t, depth),
            Ty::Bool => self.bool_expr(depth),
            Ty::Array(n, elem) => self.array(*n, elem, depth),
        }
    }

    /// An integer expression whose static range meets `t`'s, since one wholly outside it would
    /// always trap where it lands (`E0508`).
    fn narrowed(&mut self, t: IntT, depth: u32) -> E {
        self.narrowed_range(t, depth).0
    }

    /// [`Gen::narrowed`], with the expression's static range.
    fn narrowed_range(&mut self, t: IntT, depth: u32) -> (E, R) {
        for _ in 0..6 {
            let m = self.mark();
            let g = self.int_expr(Want::Any, depth);
            if g.r.meets(t.range()) && g.k.is_none_or(|v| t.range().holds(v)) {
                return (g.e, g.r);
            }
            self.restore(m);
        }
        let n = self.value_in(t.range());
        (self.literal(n, None), R::point(n))
    }

    fn array(&mut self, n: u32, elem: &Ty, depth: u32) -> E {
        let want = Ty::Array(n, Box::new(elem.clone()));
        let names: Vec<usize> = self
            .visible()
            .into_iter()
            .filter(|&id| flows(&self.binds[id].ty, &want))
            .collect();
        match self.rng.below(100) {
            0..=39 if !names.is_empty() => {
                let id = *self.rng.pick(&names);
                self.read(id)
            }
            40..=49 => match self.call(depth, |t| flows(t, &want)) {
                Some((e, _)) => e,
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
                        let base = self.read(id);
                        let i = self.index(m);
                        E::Index(Box::new(base), Box::new(i))
                    }
                }
            }
            _ => self.array_literal(n, elem, depth),
        }
    }

    fn array_literal(&mut self, n: u32, elem: &Ty, depth: u32) -> E {
        if n == 0 || self.rng.chance(30) {
            let v = self.value(elem, depth + 1);
            return E::Repeat(Box::new(v), n);
        }
        E::Array((0..n).map(|_| self.value(elem, depth + 1)).collect())
    }

    /// A value of type `ty` in a place a full expression stands: sometimes an `if`.
    ///
    /// An integer `if` takes the hull of its branches' ranges, which must lie within `u128` or
    /// within `i128` (`E0222`), so branches whose hull does not are generated again.
    fn full(&mut self, ty: &Ty, depth: u32) -> E {
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
        if self.no_calls || depth >= 3 {
            return None;
        }
        let mut options: Vec<Sig> = self
            .sigs
            .iter()
            .filter(|s| s.ret.as_ref().is_some_and(&fits))
            .filter(|s| self.cost + self.mult * s.cost <= COST_BUDGET)
            .cloned()
            .collect();
        let recursive = match (&self.current, self.recursion) {
            (Some(s), Some(_)) if !self.recursed && self.loops == 0 => {
                s.ret.as_ref().is_some_and(&fits).then(|| s.clone())
            }
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
                self.simple(&p.ty, literal)
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
    /// of one. Returns its root binding.
    fn place(&mut self, ty: Option<&Ty>) -> Option<(usize, E)> {
        let mut options: Vec<(usize, Vec<u32>)> = Vec::new();
        for id in self.visible() {
            if !matches!(self.binds[id].kind, Kind::Var | Kind::MutParam) {
                continue;
            }
            // Each depth of indexing into the binding gives a place.
            let mut t = &self.binds[id].ty;
            let mut lens = Vec::new();
            loop {
                if ty.is_none_or(|w| w == t) {
                    options.push((id, lens.clone()));
                }
                match t {
                    Ty::Array(n, inner) if *n > 0 => {
                        lens.push(*n);
                        t = inner;
                    }
                    _ => break,
                }
            }
        }
        if options.is_empty() {
            return None;
        }
        let (id, lens) = self.rng.pick(&options).clone();
        let mut e = E::name(&self.binds[id].name);
        self.exclude.push(id);
        for n in lens {
            let i = self.index(n);
            e = E::Index(Box::new(e), Box::new(i));
        }
        self.exclude.pop();
        Some((id, e))
    }

    fn place_ty(&self, id: usize, e: &E) -> Ty {
        let mut depth = 0;
        let mut x = e;
        while let E::Index(b, _) = x {
            depth += 1;
            x = b;
        }
        let mut t = self.binds[id].ty.clone();
        for _ in 0..depth {
            if let Ty::Array(_, inner) = t {
                t = *inner;
            }
        }
        t
    }

    // ---------------------------------------------------------------- statements

    fn flush(&mut self) -> Vec<S> {
        let conds = std::mem::take(&mut self.pending);
        conds
            .into_iter()
            .map(|c| {
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
                default(&t, r)
            })),
        }
    }

    /// A block of statements, the first ones given, ending with a value of type `value` when
    /// one is asked for; `clamp`, a function's contract on its result, binds the value first and
    /// returns a literal inside the contract unless a `guard` finds the value inside it too.
    fn block(&mut self, depth: u32, value: Option<&Ty>, first: Vec<S>, clamp: Option<R>) -> Block {
        self.scopes.push(Vec::new());
        let mut stmts = first;
        let count = 1 + self.rng.below(if depth == 0 { 6 } else { 4 });
        let mut ended = false;
        for _ in 0..count {
            if self.cost > COST_BUDGET {
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
                        stmts.push(S::Bind(id, ty.to_string(), e));
                        let v = E::name(&name);
                        stmts.push(S::Guard(
                            E::bin(
                                Op::And,
                                E::bin(Op::Ge, v.clone(), E::num(r.lo)),
                                E::bin(Op::Le, v.clone(), E::num(r.hi)),
                            ),
                            Block {
                                stmts: vec![S::Return(Some(default(ty, Some(r))))],
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
        Block { stmts, tail }
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
            if !self.binds[id].read && self.binds[id].kind != Kind::Param {
                self.binds[id].read = true;
                stmts.insert(at, S::Discard(E::name(&self.binds[id].name)));
            }
        }
    }

    fn statement(&mut self, depth: u32) -> Vec<S> {
        self.cost += self.mult;
        let m = self.mark();
        let roll = self.rng.below(100);
        let mut out = match roll {
            0..=29 => self.binding(depth),
            30..=49 => self.assignment(),
            50..=58 if depth < 3 => self.if_statement(depth),
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
                let leave = self.leave();
                let mut out = self.flush();
                out.push(leave);
                out
            }
            _ => self.binding(depth),
        };
        if out.is_empty() {
            self.restore(m);
            out = self.binding(depth);
        }
        out
    }

    fn binding(&mut self, depth: u32) -> Vec<S> {
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
        out.push(S::Bind(id, ty_text, init));
        out
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
        let (op, value) = match &ty {
            Ty::Int(t) if self.rng.chance(50) => {
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
                            || arithmetic(op, place_range, v.r).is_some_and(|r| r.fits());
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
        self.binds[id].assigned = true;
        let mut out = self.flush();
        out.push(S::Assign(place, op, value));
        out
    }

    fn if_statement(&mut self, depth: u32) -> Vec<S> {
        let cond = self.bool_expr(0);
        let mut out = self.flush();
        let then = self.block(depth + 1, None, Vec::new(), None);
        let els = self
            .rng
            .chance(50)
            .then(|| self.block(depth + 1, None, Vec::new(), None));
        out.push(S::If(cond, then, els));
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
        out.push(S::Bind(id, t.name.to_string(), E::Lit(init.to_string())));
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
        let body = self.block(depth + 1, None, vec![step], None);
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
        let name = format!("f{}", index);
        let refined = self.mode == Mode::Refined;
        let mut params: Vec<Param> = Vec::new();
        self.recursion = self
            .rng
            .chance(if refined { 20 } else { 10 })
            .then(|| 2 + self.rng.below(6));
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
        let ret = self.rng.chance(80).then(|| self.ty(0));
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
        self.current = Some(sig.clone());
        let mut first = Vec::new();
        if self.recursion.is_some() {
            let leave = S::Return(ret.as_ref().map(|t| default(t, ret_range)));
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
        }
        // An expression body for a small function without contracts on its result.
        let expression = first.is_empty() && ret_range.is_none() && self.rng.chance(20);
        let body = match (&ret, expression) {
            (Some(t), true) => {
                let e = self.full(t, 0);
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
        match self.rng.below(10) {
            0..=1 if !arrays.is_empty() && !t.signed => Some(Refine {
                lo: Bnd::Lit(Num::ZERO),
                hi: Bnd::Len(*self.rng.pick(&arrays)),
                inclusive: false,
            }),
            2..=3 if !names.is_empty() && !t.signed => Some(Refine {
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
                    let c = self.value_in(ta.range());
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
                out.push_str(&format!("{} {}: {} = {}\n", kw, b.name, ty, init.text()));
            }
            S::Assign(p, op, v) => out.push_str(&format!("{} {} {}\n", p.text(), op, v.text())),
            S::Discard(e) => out.push_str(&format!("_ = {}\n", e.text())),
            S::Expr(e) => out.push_str(&format!("{}\n", e.text())),
            S::If(c, then, els) => {
                out.push_str(&format!("if {} {{\n", c.text()));
                self.print_block(then, indent + 1, out);
                if let Some(e) = els {
                    out.push_str(&format!("{}}} else {{\n", pad));
                    self.print_block(e, indent + 1, out);
                }
                out.push_str(&format!("{}}}\n", pad));
            }
            S::While(c, clauses, body) => {
                out.push_str(&format!("while {}", c.text()));
                for k in clauses {
                    out.push(' ');
                    out.push_str(k);
                }
                out.push_str(" {\n");
                self.print_block(body, indent + 1, out);
                out.push_str(&format!("{}}}\n", pad));
            }
            S::Guard(c, els) => {
                out.push_str(&format!("guard {} else {{\n", c.text()));
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

/// The type at the bottom of nested arrays, and whether every array on the way is non-empty.
fn leaf(ty: &Ty) -> (&Ty, bool) {
    match ty {
        Ty::Array(0, inner) => (leaf(inner).0, false),
        Ty::Array(_, inner) => leaf(inner),
        t => (t, true),
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

/// A literal value of type `t`, inside `range` when one is given.
fn default(t: &Ty, range: Option<R>) -> E {
    match t {
        Ty::Int(_) => E::num(range.map_or(Num::ZERO, |r| r.lo)),
        Ty::Bool => E::Lit("false".into()),
        Ty::Array(n, e) => E::Repeat(Box::new(default(e, None)), *n),
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

// -------------------------------------------------------------------- arguments

/// The values a parameter takes from outside the program.
#[derive(Clone, Debug)]
enum Domain {
    Int(IntT, Option<(Bound, Bound, bool)>),
    Bool,
    Array(u32, Box<Domain>),
}

fn domain(tree: &Tree, t: TypeId) -> Option<Domain> {
    Some(match &tree.ty(t).kind {
        TypeKind::Int { name, refinement } => Domain::Int(
            IntT::named(name)?,
            refinement
                .as_ref()
                .map(|r| (r.lo.clone(), r.hi.clone(), r.inclusive)),
        ),
        TypeKind::Bool => Domain::Bool,
        TypeKind::Array { len, elem } => {
            Domain::Array(u32::try_from(*len).ok()?, Box::new(domain(tree, *elem)?))
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
    for (name, f) in module.names.iter().zip(&module.functions) {
        let Some(f) = f else { continue };
        if !public.contains(name) {
            continue;
        }
        let tree = &f.syntax.tree;
        let params: Option<Vec<(String, bool, Domain)>> = f
            .syntax
            .params
            .iter()
            .map(|p| Some((p.name.clone(), p.is_mut, domain(tree, p.ty)?)))
            .collect();
        let Some(params) = params else { continue };
        for args in function_calls(&params, &mut rng, plan) {
            out.push((name.clone(), args));
        }
    }
    out
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
            Domain::Array(..) => return None,
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

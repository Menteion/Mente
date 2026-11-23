//! `mente-smt` — the independent SMT encoding of the checks Tier 1 proves
//! ([`smt-independent-encoding`](../../docs/design-review.md#smt-independent-encoding)).
//!
//! Tier 1 erases a run-time check where it proves the check never fails, and the SMT cross-check
//! of `mente_sema::tier1::smt` writes each such proof as a query built from the very facts Tier 1
//! decided it on. That checks the decision, not the facts: a wrong fact at a loop's head passed
//! it. This crate encodes each check from the program itself, as the reference interpreter runs
//! it (`spec/core-calculus.md`, items 2 to 10, and the stack budget of item 25), and asks whether
//! some input reaches the check and fails it. A solver's `sat` is a counterexample to Tier 1's proof, which `mn smt --confirm`
//! runs on the interpreter.
//!
//! Its independence is held by the crate graph: it depends on `mente-syntax` alone, so it can
//! name nothing of Tier 1, its octagons or its facts (`test_crates_depend_only_on_allowed_crates`).
//! What it reads besides the syntax tree comes as plain data ([`Facts`]): the type the checker
//! gives each expression and the type its literals take; and each function's frame, with the
//! run's stack budget ([`Function::frame`], [`Module::budget`]), as the checker computes them for
//! every execution path to read.
//!
//! The encoding follows every path of a function from its entry, its inputs any values its
//! parameters' types, refinements and `requires` admit:
//!
//! - integers are exact, and each narrowing point, operator check, index, divisor and shift
//!   amount is checked as the interpreter checks it, in its order; a failed check ends its path;
//! - each loop runs at most [`ROUNDS`] rounds, and each function stands at most [`ROUNDS`] times
//!   on the stack of calls; a path that needs more is cut off;
//! - a call runs the callee's body, never its contract alone, so every path followed is one the
//!   program takes;
//! - a construct the encoding lacks ([`UNENCODED`]) cuts its path off, and is listed.
//!
//! So the encoding under-approximates the program: an input that satisfies a query is a run of
//! the program that fails the check, and the solver's `unsat` means only that no input does so
//! within the bounds, which is weaker than a proof.
//!
//! Each check site carries two places: the obligation Tier 1 records for it ([`Site::kind`],
//! [`Site::span`], [`Site::tag`]), which says whether Tier 1 proved it, and the trap the
//! interpreter reports when it fails ([`Site::trap`]), which `mn smt --confirm` expects.

// A match over an enum names every variant, so that a construct added to the grammar does not
// compile until the encoding decides what it does with it.
#![deny(clippy::wildcard_enum_match_arm)]

pub mod eval;
mod exec;
pub mod num;
pub mod term;

#[cfg(test)]
mod overflow_traps;

use std::fmt::Write;

use mente_syntax::tree::FnSyntax;
use mente_syntax::Span;

pub use exec::{encode, encode_within, TERMS};
use num::Big;
use term::{Terms, T};

/// How many rounds of each loop a path runs, and how many times a function stands on the stack
/// of calls, before the path is cut off.
pub const ROUNDS: usize = 3;

/// The constructs the encoding lacks, by the name it lists them under. A path that reaches one
/// is cut off there.
pub const UNENCODED: &[&str] = &[
    "a struct",
    "a field of a struct",
    "an enum",
    "a match over an enum",
    "a parameter of a struct or an enum",
    "an array of more elements than an encoding holds",
    "a call of a function without a body",
    "a function too large to encode",
];

/// The most elements an array may hold, across its nesting, to be encoded element by element:
/// a parameter's inputs, a narrowing and a comparison.
pub const ELEMENTS: u128 = 256;

/// An integer type: its name, as Tier 1's tags and the interpreter's traps write it, and its
/// range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntType {
    pub name: &'static str,
    pub signed: bool,
    pub bits: u32,
}

/// The standard integer types (`spec/core-calculus.md`, item 2).
pub const INT_TYPES: [IntType; 11] = [
    int("u8", false, 8),
    int("i8", true, 8),
    int("u16", false, 16),
    int("i16", true, 16),
    int("u32", false, 32),
    int("i32", true, 32),
    int("u64", false, 64),
    int("usize", false, 64),
    int("i64", true, 64),
    int("u128", false, 128),
    int("i128", true, 128),
];

const fn int(name: &'static str, signed: bool, bits: u32) -> IntType {
    IntType { name, signed, bits }
}

/// The type literals alone take where nothing gives them one (`spec/core-calculus.md`, item 5).
pub const I64: IntType = int("i64", true, 64);

impl IntType {
    pub fn named(name: &str) -> Option<IntType> {
        INT_TYPES.iter().copied().find(|t| t.name == name)
    }

    /// The smallest value of the type.
    pub fn min(self) -> Big {
        if self.signed {
            Big::pow2(self.bits - 1).neg()
        } else {
            Big::zero()
        }
    }

    /// The largest value of the type.
    pub fn max(self) -> Big {
        let width = if self.signed {
            self.bits - 1
        } else {
            self.bits
        };
        Big::pow2(width).sub(&Big::from_u128(1))
    }
}

/// What the checker gives an expression as its type, as the encoding reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Integers of one type, or of types that one standard type holds together.
    Int(IntType),
    /// Literals alone, which take a type from what they stand in; or integers that no standard
    /// type holds together.
    Untyped,
    Bool,
    /// An array of this length.
    Array(u128, Box<Shape>),
    /// A struct, an enum, no value, or any type.
    Other,
}

impl Shape {
    /// The integer type, where it is one.
    pub fn int(&self) -> Option<IntType> {
        match self {
            Shape::Int(t) => Some(*t),
            Shape::Untyped | Shape::Bool | Shape::Array(..) | Shape::Other => None,
        }
    }

    /// How many places a value of this shape holds, each element of each array counted, an
    /// element that holds nothing, an array of none, as one; `None` past [`ELEMENTS`].
    pub fn elements(&self) -> Option<u128> {
        let n = match self {
            Shape::Array(len, elem) => elem.elements()?.max(1).checked_mul(*len)?,
            Shape::Int(_) | Shape::Untyped | Shape::Bool | Shape::Other => 1,
        };
        (n <= ELEMENTS).then_some(n)
    }
}

/// What the checker knows of one function, besides its syntax.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Each expression's type, by its index.
    pub exprs: Vec<Shape>,
    /// For each expression of literals alone, by its index, the type its literals take
    /// (`spec/core-calculus.md`, item 5).
    pub literals: Vec<Option<IntType>>,
    /// For each statement that declares a binding, by its index, the binding's type: the one
    /// it states, or its initializer's.
    pub bindings: Vec<Option<Shape>>,
    /// For each expression, by its index, whether its integers have different types that one
    /// standard type holds: its `Shape::Int` is then that type.
    pub mixed: Vec<bool>,
}

/// A function with a body, and what the checker knows of it.
#[derive(Clone, Debug)]
pub struct Function<'a> {
    pub syntax: &'a FnSyntax,
    pub facts: Facts,
    /// The bytes a call of it counts against the stack budget (`spec/core-calculus.md`, item
    /// 25), as the checker computes them for every execution path to read.
    pub frame: u128,
}

/// The functions of one module, in its order: each with a body, or `None` for a declaration
/// without one.
#[derive(Clone, Debug)]
pub struct Module<'a> {
    pub functions: Vec<Option<Function<'a>>>,
    /// The run's stack budget (`spec/core-calculus.md`, item 25): a call traps where the frames
    /// of the calls begun and not returned and the callee's together exceed it.
    pub budget: u128,
}

impl Module<'_> {
    /// The index of the function named `name`, with a body or not.
    pub fn index(&self, name: &str) -> Option<usize> {
        self.functions
            .iter()
            .position(|f| f.as_ref().is_some_and(|f| f.syntax.name == name))
    }
}

/// A kind of check, as Tier 1 names the obligation it records for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Refinement,
    Range,
    Index,
    Divisor,
    Shift,
    Requires,
    Ensures,
    Invariant,
    Decreases,
}

impl Kind {
    /// The kind as one word, as `mn smt` names a query.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Refinement => "refinement",
            Kind::Range => "range",
            Kind::Index => "index",
            Kind::Divisor => "divisor",
            Kind::Shift => "shift",
            Kind::Requires => "requires",
            Kind::Ensures => "ensures",
            Kind::Invariant => "invariant",
            Kind::Decreases => "decreases",
        }
    }
}

/// What tells apart Tier 1's obligations of one kind at one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    None,
    /// A narrowing into this type.
    Into(&'static str),
    /// A contract clause, an invariant's or a measure that is never negative, by its span.
    Clause(Span),
    /// The `decreases` measures together, smaller than before.
    Smaller,
    /// A `mut` place's refinement, with what the callee left in it.
    Back,
}

/// Where the reference interpreter reports a check that fails: the trap's kind, as
/// `TrapKind::name` writes it, and its span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    pub kind: &'static str,
    pub span: Span,
}

/// A check of the encoded function: the obligation Tier 1 records for it, the trap it raises,
/// and the condition under which an input reaches it and fails it.
#[derive(Clone, Debug)]
pub struct Site {
    pub kind: Kind,
    pub span: Span,
    pub tag: Tag,
    pub trap: Trap,
    /// The path reaches the check and the check fails.
    pub fails: T,
    /// Some path was cut off past the bounds before the check: one that would reach it may be
    /// among those, so a check no input reaches is not shown unreachable.
    pub beyond_bound: bool,
    /// Some path was cut off at a construct the encoding lacks before the check.
    pub beyond_lacked: bool,
}

impl Site {
    /// What tells this obligation apart from Tier 1's others.
    pub fn key(&self) -> (Kind, Span, Tag) {
        (self.kind, self.span, self.tag)
    }

    /// The obligation as a query names it ([`obligation_text`]).
    pub fn obligation(&self) -> String {
        obligation_text(self.key())
    }
}

/// An obligation as a query names it, from its key: its kind, its place and its tag, such as
/// `range at 4:19 into `i32``. A check with no site is named by the same text.
pub fn obligation_text((kind, span, tag): (Kind, Span, Tag)) -> String {
    format!(
        "{} at {}:{}{}",
        kind.name(),
        span.line,
        span.col,
        tag_text(tag)
    )
}

/// A parameter of the encoded function and the inputs that give its value: one for an integer
/// or a boolean, one for each element of an array, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub shape: Shape,
    pub inputs: Vec<String>,
}

/// The encoding of one function.
#[derive(Clone, Debug)]
pub struct Encoding {
    pub function: String,
    pub terms: Terms,
    pub params: Vec<Param>,
    /// The function's checks, those of the functions it calls left out: each place of each
    /// round of each loop is a site of its own.
    pub sites: Vec<Site>,
    /// Each construct the encoding lacks where a path reaches it, with its place.
    pub unencoded: Vec<(Span, &'static str)>,
    /// Each place where a path ends past the stack budget (`spec/core-calculus.md`, item 25): a
    /// call whose frames exceed it, or the function's name, where its own frame does. A path
    /// ended there is neither a site nor a cut, so that checks past it are no query; the export
    /// lists each place, so that a wrong count shows.
    pub overflowed: Vec<Span>,
    /// The inputs lie in their parameters' types and refinements, and the function's `requires`
    /// and measures hold of them, as the run reaches those checks: for a function whose own frame
    /// passes the stack budget, no input, since the run traps at its name first.
    pub admitted: T,
    /// Some path of the inputs is cut off: by a loop or a call past the bounds, or by a
    /// construct the encoding lacks.
    pub cut: T,
    /// Some path is cut off past the bounds, anywhere in the function, before or after any of
    /// its sites: a check whose sites never fail is unreachable only where no path is cut.
    pub cut_past_bound: bool,
    /// Some path is cut off at a construct the encoding lacks, anywhere in the function.
    pub cut_at_lacked: bool,
    /// The run of the inputs returns, on a path the encoding follows, without a trap.
    pub returns: T,
    /// The value it returns, where the function has one.
    pub result: Option<T>,
    /// Each parameter's value when the run returns, in the parameters' order: what a `mut`
    /// parameter copies back to its caller's place.
    pub finals: Vec<Option<T>>,
}

impl Encoding {
    /// A query of the condition `goal` alone: its inputs declared, its terms defined, and the
    /// condition asserted, for [`eval::holds`] to read under given inputs.
    pub fn assertion(&self, goal: T) -> String {
        let mut out = String::new();
        self.terms.write(&[goal], &mut out);
        let _ = writeln!(out, "(assert {})", self.terms.atom(goal));
        out
    }

    /// The query of sites `ks`, which share one obligation and one trap: a solver answers `sat`
    /// with the inputs of a run that fails the check at one of them, or `unsat`. `None` where no
    /// input reaches the check, or `ks` is empty. Its comments name the function, the obligation
    /// and the trap a run of the model must raise.
    pub fn query(&mut self, ks: &[usize]) -> Option<String> {
        let first = *ks.first()?;
        let each: Vec<T> = ks.iter().map(|&k| self.sites[k].fails).collect();
        let fails = self.terms.or(&each);
        if self.terms.as_bool(fails) == Some(false) {
            return None;
        }
        let s = &self.sites[first];
        let mut out = String::new();
        let _ = writeln!(out, "; function: {}", self.function);
        let _ = writeln!(out, "; obligation: {}", s.obligation());
        let _ = writeln!(
            out,
            "; trap: {} at {}:{}",
            s.trap.kind, s.trap.span.line, s.trap.span.col
        );
        out.push_str(&self.assertion(fails));
        out.push_str("(check-sat)\n");
        let read = self.terms.reachable(&[fails]);
        let inputs: Vec<String> = read
            .iter()
            .filter_map(|&t| match self.terms.node(t) {
                term::Node::Input(name, _) => Some(name.clone()),
                term::Node::Num(_)
                | term::Node::Bool(_)
                | term::Node::App(..)
                | term::Node::Const(..) => None,
            })
            .collect();
        if !inputs.is_empty() {
            let _ = writeln!(out, "(get-value ({}))", inputs.join(" "));
        }
        Some(out)
    }
}

/// A tag as a query's comment writes it.
fn tag_text(tag: Tag) -> String {
    match tag {
        Tag::None => String::new(),
        Tag::Into(t) => format!(" into `{}`", t),
        Tag::Clause(s) => format!(" of the clause at {}:{}", s.line, s.col),
        Tag::Smaller => " smaller".to_string(),
        Tag::Back => " back from the call".to_string(),
    }
}

/// The function, the obligation and the trap's kind and place a query's comments name. The
/// obligation is its kind, its place and its tag, `range at 4:19 into `i32``, which tells apart
/// the obligations that share one trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub function: String,
    pub obligation: String,
    pub trap_kind: String,
    pub trap_line: u32,
    pub trap_col: u32,
}

/// The arithmetic a query's terms use, which decides how hard it is for a solver, from the
/// least: `linear` integer arithmetic, its divisions and remainders by numerals among it;
/// `nonlinear`, with a product of two terms of no numeral, or a division or a remainder by one;
/// and `bit-vector`, with a conversion between integers and bit-vectors.
pub fn fragment(query: &str) -> &'static str {
    let Ok(exprs) = eval::parse(query) else {
        return "unread";
    };
    fn walk(s: &eval::Sexp, nonlinear: &mut bool, bits: &mut bool) {
        let eval::Sexp::List(items) = s else {
            return;
        };
        let numeral = |s: &eval::Sexp| match s {
            eval::Sexp::Atom(a) => a.bytes().all(|b| b.is_ascii_digit()),
            eval::Sexp::List(l) => {
                matches!(l.as_slice(), [eval::Sexp::Atom(m), eval::Sexp::Atom(n)]
                if m == "-" && n.bytes().all(|b| b.is_ascii_digit()))
            }
        };
        match items.as_slice() {
            [eval::Sexp::Atom(op), a, b] if op == "*" => {
                *nonlinear |= !numeral(a) && !numeral(b);
            }
            [eval::Sexp::Atom(op), _, b] if op == "div" || op == "mod" => {
                *nonlinear |= !numeral(b);
            }
            [eval::Sexp::Atom(op), ..] if op == "bv2nat" => *bits = true,
            _ => {}
        }
        for item in items {
            walk(item, nonlinear, bits);
        }
    }
    let (mut nonlinear, mut bits) = (false, false);
    for e in &exprs {
        walk(e, &mut nonlinear, &mut bits);
    }
    if bits {
        "bit-vector"
    } else if nonlinear {
        "nonlinear"
    } else {
        "linear"
    }
}

/// What a query's comments say of it, as [`Encoding::query`] writes them.
pub fn header(query: &str) -> Option<Header> {
    let mut function = None;
    let mut obligation = None;
    let mut trap = None;
    for line in query.lines() {
        if let Some(rest) = line.strip_prefix("; function: ") {
            function = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("; obligation: ") {
            obligation = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("; trap: ") {
            let (kind, place) = rest.split_once(" at ")?;
            let (l, c) = place.split_once(':')?;
            trap = Some((kind.to_string(), l.parse().ok()?, c.parse().ok()?));
        }
    }
    let (trap_kind, trap_line, trap_col) = trap?;
    Some(Header {
        function: function?,
        obligation: obligation?,
        trap_kind,
        trap_line,
        trap_col,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_types_name_their_ranges() {
        let u8t = IntType::named("u8").unwrap();
        assert_eq!((u8t.min(), u8t.max()), (Big::zero(), Big::from_u128(255)));
        let i8t = IntType::named("i8").unwrap();
        assert_eq!(
            (i8t.min(), i8t.max()),
            (Big::from_i128(-128), Big::from_i128(127))
        );
        let u128t = IntType::named("u128").unwrap();
        assert_eq!(u128t.max(), Big::from_u128(u128::MAX));
        let i128t = IntType::named("i128").unwrap();
        assert_eq!(i128t.min(), Big::from_i128(i128::MIN));
        assert_eq!(
            IntType::named("usize").map(|t| (t.signed, t.bits)),
            Some((false, 64))
        );
        assert_eq!(IntType::named("u7"), None);
    }

    #[test]
    fn test_a_shape_counts_its_elements_up_to_the_limit() {
        let u8t = Shape::Int(IntType::named("u8").unwrap());
        let row = Shape::Array(16, Box::new(u8t.clone()));
        assert_eq!(row.elements(), Some(16));
        assert_eq!(
            Shape::Array(16, Box::new(row.clone())).elements(),
            Some(256)
        );
        assert_eq!(Shape::Array(17, Box::new(row)).elements(), None);
        assert_eq!(
            Shape::Array(u128::MAX, Box::new(Shape::Array(2, Box::new(u8t.clone())))).elements(),
            None
        );
        assert_eq!(u8t.elements(), Some(1));
        // An array of arrays of no element still has its slots to count.
        let none = Shape::Array(0, Box::new(u8t.clone()));
        assert_eq!(none.elements(), Some(0));
        assert_eq!(Shape::Array(300, Box::new(none.clone())).elements(), None);
        assert_eq!(Shape::Array(256, Box::new(none)).elements(), Some(256));
        assert_eq!(u8t.int().map(|t| t.name), Some("u8"));
        assert_eq!(Shape::Bool.int(), None);
    }

    /// A query's fragment is the hardest arithmetic its terms use: a product of two terms of no
    /// numeral, or a division or a remainder by one, is nonlinear; a bit-vector conversion is
    /// harder still; divisions by numerals and products with one stay linear.
    #[test]
    fn test_a_query_names_the_arithmetic_it_uses() {
        let q = |body: &str| {
            format!("(declare-const p.x Int)\n(declare-const p.y Int)\n(define-fun t1 () Int {})\n(assert (< t1 0))\n", body)
        };
        for (body, named) in [
            ("(+ p.x (* 3 p.y))", "linear"),
            ("(* p.x (- 2))", "linear"),
            ("(div p.x 4)", "linear"),
            ("(mod p.x (- 4))", "linear"),
            ("(* p.x p.y)", "nonlinear"),
            ("(div p.x p.y)", "nonlinear"),
            ("(mod 7 p.y)", "nonlinear"),
            (
                "(bv2nat (bvand ((_ int2bv 8) p.x) ((_ int2bv 8) p.y)))",
                "bit-vector",
            ),
            (
                "(+ (* p.x p.y) (bv2nat (bvnot ((_ int2bv 8) p.x))))",
                "bit-vector",
            ),
        ] {
            assert_eq!(fragment(&q(body)), named, "{}", body);
        }
        assert_eq!(fragment("(assert"), "unread");
    }

    #[test]
    fn test_a_query_header_names_the_function_and_the_trap() {
        let q = "; source: a.mn:3:4\n; function: f\n; obligation: divisor at 3:9\n; trap: division-by-zero at 3:5\n(assert true)\n";
        assert_eq!(
            header(q),
            Some(Header {
                function: "f".into(),
                obligation: "divisor at 3:9".into(),
                trap_kind: "division-by-zero".into(),
                trap_line: 3,
                trap_col: 5,
            })
        );
        assert_eq!(header("; function: f\n; trap: x at 1\n"), None);
        assert_eq!(header("; trap: x at 1:2\n"), None);
        for (tag, text) in [
            (Tag::None, ""),
            (Tag::Into("u8"), " into `u8`"),
            (Tag::Smaller, " smaller"),
            (Tag::Back, " back from the call"),
        ] {
            assert_eq!(tag_text(tag), text);
        }
        let s = Span {
            start: 0,
            end: 1,
            line: 2,
            col: 3,
        };
        assert_eq!(tag_text(Tag::Clause(s)), " of the clause at 2:3");
    }
}

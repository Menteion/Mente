//! `mente-interp` — the reference interpreter of Mente (`spec/core-calculus.md`).
//!
//! It runs a module that passed every check, over the typed tree that `mente-sema` hands it,
//! rule by rule as the core calculus states them: exact integer arithmetic, narrowing at the
//! points the spec names, the traps of item 9, and evaluation from left to right. It shares
//! no code with MIR or native code, so the three execution paths check one another
//! ([`reference-interpreter-crate`](../../docs/design-review.md#reference-interpreter-crate)).
//!
//! It also checks at run time every refinement and contract that Tier 1 must prove: a value
//! entering a refined type, `requires` at a call, `ensures` at a return, and each loop
//! `invariant` and `decreases` at the loop head. A program Tier 1 accepts never trips them, so
//! the interpreter judges Tier 1 independently
//! ([`interpreter-checks-contracts`](../../docs/design-review.md#interpreter-checks-contracts)).

// A match over an enum names every variant, so that a variant added later does not compile until
// each match decides what it does with it; a `_` arm once let walkers skip expression kinds
// ([`guard-per-problem`](../../docs/design-review.md#guard-per-problem)). A match that is a
// classifier by nature keeps `#[expect]`, with the reason every variant belongs in its arm.
#![deny(clippy::wildcard_enum_match_arm)]

mod eval;
pub mod int;

use std::fmt;

use mente_sema::CheckedModule;
use mente_syntax::Span;

pub use int::Int;

/// A value of M2's slice: an integer, a `bool`, an array, a struct, a variant of an enum, or no
/// value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(Int),
    Bool(bool),
    Array(Vec<Value>),
    /// A struct by its name, with its fields' names and values in the order of its declaration.
    Struct(String, Vec<(String, Value)>),
    /// A variant by its enum's name and its own, with its payload's fields' names and values in
    /// the order of its declaration, none for a variant that holds nothing.
    Enum(String, String, Vec<(String, Value)>),
    Unit,
}

/// The canonical form of a value, Mente's own syntax for it (`core-calculus.md`, item 23):
/// `42`, `-1`, `true`, `[1, 2, 3]`, `Pair(i: 0, j: 1)`, `Shape.Dot`, `Shape.Rect(w: 3, h: 4)`, `()`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Struct(name, fields) => {
                write!(f, "{}(", name)?;
                for (i, (n, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, v)?;
                }
                write!(f, ")")
            }
            Value::Enum(name, variant, fields) => {
                write!(f, "{}.{}", name, variant)?;
                if fields.is_empty() {
                    return Ok(());
                }
                write!(f, "(")?;
                for (i, (n, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, v)?;
                }
                write!(f, ")")
            }
            Value::Unit => write!(f, "()"),
        }
    }
}

/// What made a program trap (`core-calculus.md`, item 9), and the run-time checks of what
/// Tier 1 must prove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrapKind {
    /// A value outside the integer type it is narrowed into; `None` when the exact value needs
    /// more than 128 bits.
    Narrowing {
        value: Option<Int>,
        into: &'static str,
    },
    /// A value outside the refined range it enters.
    Refinement {
        value: Int,
        range: String,
    },
    DivisionByZero,
    IndexOutOfBounds {
        index: Int,
        len: usize,
    },
    ShiftAmount {
        amount: Int,
        width: u32,
    },
    Todo(String),
    Unreachable(String),
    /// A `requires` clause false at a call.
    Requires(String),
    /// An `ensures` clause false at a return.
    Ensures(String),
    /// A loop `invariant` false at the loop head.
    Invariant(String),
    /// A `decreases` measure negative, or not smaller than at the round or call before.
    Decreases(String),
    StackOverflow,
    /// An intermediate result whose magnitude needs more than 128 bits.
    IntermediateOverflow,
}

impl TrapKind {
    /// The kind's stable name, as `mn run` prints it.
    pub fn name(&self) -> &'static str {
        match self {
            TrapKind::Narrowing { .. } => "narrowing",
            TrapKind::Refinement { .. } => "refinement",
            TrapKind::DivisionByZero => "division-by-zero",
            TrapKind::IndexOutOfBounds { .. } => "index-out-of-bounds",
            TrapKind::ShiftAmount { .. } => "shift-amount",
            TrapKind::Todo(_) => "todo",
            TrapKind::Unreachable(_) => "unreachable",
            TrapKind::Requires(_) => "requires",
            TrapKind::Ensures(_) => "ensures",
            TrapKind::Invariant(_) => "invariant",
            TrapKind::Decreases(_) => "decreases",
            TrapKind::StackOverflow => "stack-overflow",
            TrapKind::IntermediateOverflow => "intermediate-overflow",
        }
    }
}

impl fmt::Display for TrapKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrapKind::Narrowing {
                value: Some(value),
                into,
            } => write!(f, "{} does not fit in `{}`", value, into),
            TrapKind::Narrowing { value: None, into } => {
                write!(f, "the result does not fit in `{}`", into)
            }
            TrapKind::Refinement { value, range } => {
                write!(f, "{} lies outside the refinement `[{}]`", value, range)
            }
            TrapKind::DivisionByZero => write!(f, "division by zero"),
            TrapKind::IndexOutOfBounds { index, len } => {
                write!(f, "index {} is outside an array of length {}", index, len)
            }
            TrapKind::ShiftAmount { amount, width } => {
                write!(f, "shift amount {} is outside 0 ..= {}", amount, width - 1)
            }
            TrapKind::Todo(reason) => write!(f, "todo reached: {}", reason),
            TrapKind::Unreachable(reason) => write!(f, "unreachable reached: {}", reason),
            TrapKind::Requires(clause) => write!(f, "`requires {}` is false at this call", clause),
            TrapKind::Ensures(clause) => write!(f, "`ensures {}` is false at this return", clause),
            TrapKind::Invariant(clause) => {
                write!(f, "`invariant {}` is false at the loop head", clause)
            }
            TrapKind::Decreases(measure) => {
                write!(f, "`decreases {}` is negative or did not decrease", measure)
            }
            TrapKind::StackOverflow => write!(f, "the call stack ran out"),
            TrapKind::IntermediateOverflow => {
                write!(f, "an intermediate result needs more than 128 bits")
            }
        }
    }
}

/// A trap and the span of the operation that trapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    pub kind: TrapKind,
    pub span: Span,
}

/// What the message of a broken run, [`Stop::Broken`], starts with: the comparison of the
/// execution paths reads it to blame the reference interpreter rather than the path it is
/// compared with.
pub const BROKEN: &str = "broken interpreter state";

/// Why a run stopped without a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Trap(Trap),
    /// The run took more steps than its limit allows.
    OutOfSteps,
    /// An array literal asked for more elements than the limit allows.
    TooLarge(u128),
    /// No function of that name has a body to run.
    NoFunction(String),
    /// The arguments do not match the function's parameters.
    BadArguments(String),
    /// The run reached a state the checks rule out, such as an operand of no integer where one
    /// is read: a bug of the compiler, never of the program.
    Broken(String),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Trap(t) => write!(
                f,
                "trap[{}] at {}:{}: {}",
                t.kind.name(),
                t.span.line,
                t.span.col,
                t.kind
            ),
            Stop::OutOfSteps => write!(f, "the run took more steps than its limit"),
            Stop::TooLarge(n) => write!(f, "an array of {} elements is over the limit", n),
            Stop::NoFunction(name) => write!(f, "no function `{}` with a body to run", name),
            Stop::BadArguments(why) => write!(f, "{}", why),
            Stop::Broken(why) => write!(f, "{}: {}", BROKEN, why),
        }
    }
}

/// The resources a run may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Nested calls before the stack runs out, which traps.
    pub max_depth: usize,
    /// Statements and loop rounds before the run stops.
    pub max_steps: u64,
    /// Elements of one array literal `[v; n]`.
    pub max_array: u128,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_depth: 10_000,
            max_steps: 100_000_000,
            max_array: 1 << 24,
        }
    }
}

/// Calls the function `name` of a checked module with `args`, each narrowed into its
/// parameter's type, and returns its result.
pub fn call(
    module: &CheckedModule,
    name: &str,
    args: Vec<Value>,
    limits: Limits,
) -> Result<Value, Stop> {
    call_leaving(module, name, args, limits).map(|(v, _)| v)
}

/// Calls the function `name` like [`call`], and returns its result with the final value of
/// each parameter, which a `mut` parameter leaves in the place its caller passed. The run gets
/// a thread of its own with a stack deep enough for `limits.max_depth` calls.
pub fn call_leaving(
    module: &CheckedModule,
    name: &str,
    args: Vec<Value>,
    limits: Limits,
) -> Result<(Value, Vec<Value>), Stop> {
    run(module, name, args, limits, None)
}

/// What a run shows of its values: each integer an expression gives, an array's elements
/// included, with the expression's span, as it is evaluated; the value a compound assignment
/// computes, `x op e`, which no expression gives, with the span of the assignment; and each
/// integer a narrowing point narrows, with the span the narrowing traps at, as it enters the
/// type and before it is checked there ([`Shown`]).
pub type Observer<'a> = &'a mut (dyn FnMut(Span, Int, Shown) + Send);

/// Why a run shows its observer an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// An expression gave it, or a compound assignment computed it.
    Given,
    /// A narrowing point narrows it (`core-calculus.md`, item 4). A value is not always
    /// narrowed as soon as it is given: a call's arguments are all evaluated before the first
    /// is narrowed, so a later argument's value may never reach its narrowing.
    Narrowed,
}

/// Calls the function `name` like [`call_leaving`], showing `seen` each integer an expression
/// gives, exact, before it is narrowed anywhere, and each integer a narrowing point narrows,
/// in the order of the run and up to the trap that ends it. A claim about the values a
/// narrowing takes, such as Tier 1's that it always traps (`E0508`), is checked against them.
pub fn call_observed<'m>(
    module: &'m CheckedModule,
    name: &str,
    args: Vec<Value>,
    limits: Limits,
    seen: Observer<'m>,
) -> Result<(Value, Vec<Value>), Stop> {
    run(module, name, args, limits, Some(seen))
}

fn run<'m>(
    module: &'m CheckedModule,
    name: &str,
    args: Vec<Value>,
    limits: Limits,
    seen: Option<Observer<'m>>,
) -> Result<(Value, Vec<Value>), Stop> {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(stack_bytes(limits.max_depth))
            .spawn_scoped(s, || {
                eval::Machine::new(module, limits, seen).enter(name, args)
            })
            .map_err(|e| Stop::BadArguments(format!("the interpreter could not start: {}", e)))?
            .join()
            .unwrap_or_else(|_| {
                Err(Stop::BadArguments(
                    "the interpreter stopped unexpectedly".to_string(),
                ))
            })
    })
}

/// Stack bytes reserved for each nested call of the interpreted program.
const STACK_PER_CALL: usize = 64 << 10;

/// The stack a run's thread gets: a base for the interpreter itself, and room for each call.
fn stack_bytes(max_depth: usize) -> usize {
    (1 << 20) + STACK_PER_CALL * max_depth
}

/// Runs a program: its `main`, which takes no parameters (`core-calculus.md`, item 10).
pub fn run_main(module: &CheckedModule, limits: Limits) -> Result<Value, Stop> {
    call(module, "main", Vec::new(), limits)
}

/// Reads a call written as `name(arg, ...)`, whose arguments are integer, `bool`, array and
/// struct values in the form a value prints in (`core-calculus.md`, item 23), such as
/// `clamp(-3, 0, 10)`, `sum([1, 2, 3])` or `norm(Point(x: 3, y: -4))`.
pub fn parse_call(text: &str) -> Option<(String, Vec<Value>)> {
    parse_call_modes(text).map(|(name, args)| (name, args.into_iter().map(|(_, v)| v).collect()))
}

/// Reads a call like [`parse_call`], where an argument may be marked `mut`, as in
/// `reverse(mut [1, 2, 3])`; each argument comes with whether it is.
pub fn parse_call_modes(text: &str) -> Option<(String, Vec<(bool, Value)>)> {
    let text = text.trim();
    let open = text.find('(')?;
    let name = text[..open].trim();
    let inner = text[open + 1..].strip_suffix(')')?;
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return None;
    }
    let mut reader = Reader {
        chars: inner.chars().collect(),
        pos: 0,
    };
    let mut args = Vec::new();
    reader.skip_space();
    if reader.pos < reader.chars.len() {
        loop {
            reader.skip_space();
            let rest: String = reader.chars[reader.pos..].iter().take(4).collect();
            let is_mut = rest == "mut ";
            if is_mut {
                reader.pos += 4;
            }
            args.push((is_mut, reader.value()?));
            reader.skip_space();
            match reader.next() {
                Some(',') => continue,
                None => break,
                Some(_) => return None,
            }
        }
    }
    Some((name.to_string(), args))
}

/// A variant written with empty parentheses in a call's text, as `M.Z()`, which reads as no
/// value: a variant that holds nothing is written without them, `M.Z`. The first one, if any,
/// so that a refused call can say so.
pub fn empty_payload(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let word = |c: &char| c.is_ascii_alphanumeric() || *c == '_';
    for (k, _) in chars.iter().enumerate().filter(|(_, c)| **c == '(') {
        let mut end = k + 1;
        while chars.get(end).is_some_and(|c| c.is_whitespace()) {
            end += 1;
        }
        if chars.get(end) != Some(&')') {
            continue;
        }
        let mut name_end = k;
        while name_end > 0 && chars[name_end - 1].is_whitespace() {
            name_end -= 1;
        }
        let mut start = name_end;
        while start > 0 && word(&chars[start - 1]) {
            start -= 1;
        }
        if start == name_end || start == 0 || chars[start - 1] != '.' {
            continue;
        }
        let mut enum_start = start - 1;
        while enum_start > 0 && word(&chars[enum_start - 1]) {
            enum_start -= 1;
        }
        if enum_start == start - 1 {
            continue;
        }
        return Some(chars[enum_start..name_end].iter().collect());
    }
    None
}

struct Reader {
    chars: Vec<char>,
    pos: usize,
}

impl Reader {
    fn skip_space(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_space();
        match self.chars.get(self.pos)? {
            '[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_space();
                if self.chars.get(self.pos) == Some(&']') {
                    self.pos += 1;
                    return Some(Value::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.skip_space();
                    match self.next()? {
                        ',' => continue,
                        ']' => return Some(Value::Array(items)),
                        _ => return None,
                    }
                }
            }
            c if c.is_ascii_alphabetic() || *c == '_' => {
                let start = self.pos;
                while self
                    .chars
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    self.pos += 1;
                }
                let word: String = self.chars[start..self.pos].iter().collect();
                match word.as_str() {
                    "true" => return Some(Value::Bool(true)),
                    "false" => return Some(Value::Bool(false)),
                    _ => {}
                }
                // A variant: `Name.A`, or `Name.B(field: value, ...)`, every field named.
                if self.chars.get(self.pos) == Some(&'.') {
                    self.pos += 1;
                    let start = self.pos;
                    while self
                        .chars
                        .get(self.pos)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        self.pos += 1;
                    }
                    let variant: String = self.chars[start..self.pos].iter().collect();
                    if variant.is_empty() || variant.starts_with(|c: char| c.is_ascii_digit()) {
                        return None;
                    }
                    // A space may stand before the payload's `(`, as before a struct's.
                    let after = self.pos;
                    self.skip_space();
                    if self.chars.get(self.pos) != Some(&'(') {
                        self.pos = after;
                        return Some(Value::Enum(word, variant, Vec::new()));
                    }
                    self.pos += 1;
                    let fields = self.fields()?;
                    return Some(Value::Enum(word, variant, fields));
                }
                // A struct: `Name(field: value, ...)`, every field named.
                self.skip_space();
                if self.next()? != '(' {
                    return None;
                }
                self.fields().map(|fields| Value::Struct(word, fields))
            }
            _ => {
                let negative = self.chars.get(self.pos) == Some(&'-');
                if negative {
                    self.pos += 1;
                }
                let start = self.pos;
                while self
                    .chars
                    .get(self.pos)
                    .is_some_and(|c| c.is_ascii_digit() || *c == '_')
                {
                    self.pos += 1;
                }
                let digits: String = self.chars[start..self.pos]
                    .iter()
                    .filter(|c| **c != '_')
                    .collect();
                let mag: u128 = digits.parse().ok()?;
                let v = Int::from_u128(mag);
                Some(Value::Int(if negative { v.negate() } else { v }))
            }
        }
    }

    /// Named fields after their `(`, to the `)` that closes them: `field: value, ...`.
    fn fields(&mut self) -> Option<Vec<(String, Value)>> {
        let mut fields = Vec::new();
        loop {
            self.skip_space();
            let start = self.pos;
            while self
                .chars
                .get(self.pos)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                self.pos += 1;
            }
            let field: String = self.chars[start..self.pos].iter().collect();
            if field.is_empty() || field.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            self.skip_space();
            if self.next()? != ':' {
                return None;
            }
            fields.push((field, self.value()?));
            self.skip_space();
            match self.next()? {
                ',' => continue,
                ')' => return Some(fields),
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod overflow_traps;

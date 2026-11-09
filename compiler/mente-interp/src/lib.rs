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

mod eval;
pub mod int;

use std::fmt;

use mente_sema::CheckedModule;
use mente_syntax::Span;

pub use int::Int;

/// A value of M1: an integer, a `bool`, an array, or no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(Int),
    Bool(bool),
    Array(Vec<Value>),
    Unit,
}

/// The canonical form of a value: `42`, `-1`, `true`, `[1, 2, 3]`, `()`.
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
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size((1 << 20) + STACK_PER_CALL * limits.max_depth)
            .spawn_scoped(s, || eval::Machine::new(module, limits).enter(name, args))
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

/// Runs a program: its `main`, which takes no parameters (`core-calculus.md`, item 10).
pub fn run_main(module: &CheckedModule, limits: Limits) -> Result<Value, Stop> {
    call(module, "main", Vec::new(), limits)
}

/// Reads a call written as `name(arg, ...)`, whose arguments are integer, `bool` and array
/// literals, such as `clamp(-3, 0, 10)` or `sum([1, 2, 3])`.
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
            't' | 'f' => {
                let rest: String = self.chars[self.pos..].iter().take(5).collect();
                if rest.starts_with("true") {
                    self.pos += 4;
                    Some(Value::Bool(true))
                } else if rest.starts_with("false") {
                    self.pos += 5;
                    Some(Value::Bool(false))
                } else {
                    None
                }
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
}

#[cfg(test)]
mod tests;

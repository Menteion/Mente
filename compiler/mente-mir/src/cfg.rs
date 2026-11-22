//! MIR as an SSA control-flow graph with block parameters, stored as structure of arrays
//! (`spec/mir-and-codegen.md` item 1, [`mir-control-flow`]).
//!
//! A [`Function`] keeps one row per value, an instruction's result or a block's parameter, in
//! parallel arrays addressed by [`NodeId`]: its operation, its type, its operands (a range of a
//! shared pool) and its source location. Blocks list their parameters and instructions, and end
//! in one terminator. Arrays live in memory: a stack slot, or an address a caller passes, read
//! and written by `load` and `store`. A `mut` parameter is an address too.
//!
//! Every value MIR computes fits its type. An exact operation's operands and result share one
//! type, which the lowering picks wide enough from the static ranges of `core-calculus.md`
//! item 4, so it never overflows; a checked one, on `i128` or `u128`, traps. Each run-time check
//! is an instruction of its own, so a pass can erase one that Tier 1 proved, and each trap site
//! carries the message the reference interpreter prints, so the three execution paths can be
//! compared line for line.
//!
//! [`mir-control-flow`]: ../../docs/design-review.md#mir-control-flow

use std::fmt;
use std::ops::Range;

use crate::NodeId;

/// A block of a function, by index; block 0 is the entry, whose parameters are the function's.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);

/// A function of a module, by index.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FuncId(pub u32);

/// A trap site of a function, by index.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrapId(pub u32);

/// A fact of a function's table, by index ([`Fact`]): kept apart, as a trap site is, so that an
/// operation that states one stays as small as the others.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FactId(pub u32);

/// An integer type: its width in bits (8, 16, 32, 64 or 128) and whether it is signed.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct IntTy {
    pub bits: u8,
    pub signed: bool,
}

impl IntTy {
    pub const fn new(bits: u8, signed: bool) -> IntTy {
        IntTy { bits, signed }
    }

    /// The type's smallest value.
    pub fn min(self) -> Num {
        if self.signed {
            Num::new(true, 1 << (self.bits - 1))
        } else {
            Num::ZERO
        }
    }

    /// The type's largest value.
    pub fn max(self) -> Num {
        Num::new(
            false,
            match (self.signed, self.bits) {
                (false, 128) => u128::MAX,
                (false, b) => (1 << b) - 1,
                (true, b) => (1 << (b - 1)) - 1,
            },
        )
    }

    pub fn holds(self, n: Num) -> bool {
        self.min() <= n && n <= self.max()
    }

    /// Bytes in memory.
    pub fn bytes(self) -> u32 {
        u32::from(self.bits) / 8
    }
}

impl fmt::Display for IntTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", if self.signed { 'i' } else { 'u' }, self.bits)
    }
}

/// A value's type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Ty {
    Int(IntTy),
    Bool,
    /// The address of memory of a shape ([`Mem`]): a stack slot, an element or a field of one,
    /// or what a caller passed. Two addresses have one type only where they point at memory of
    /// one shape, so a join, a call and a copy agree on it, and the verifier holds every slot,
    /// element, load and store to it ([`verify`]).
    Addr(MemId),
    /// What an instruction without a result has.
    Unit,
    /// The latest effect where a block begins: the type of its last parameter, and of no other,
    /// which a jump or a call fills with the latest effect where it stands ([`Chain`]). Native
    /// code keeps no value for it.
    Effect,
}

impl Ty {
    /// The shape of the memory an address points at; none for a value that is no address.
    pub fn mem(self) -> Option<MemId> {
        match self {
            Ty::Addr(m) => Some(m),
            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Effect => None,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int(t) => write!(f, "{}", t),
            Ty::Bool => write!(f, "bool"),
            Ty::Addr(_) => write!(f, "addr"),
            Ty::Unit => write!(f, "unit"),
            Ty::Effect => write!(f, "effect"),
        }
    }
}

/// What memory holds at an address, so its every access: an integer, a boolean, an array of
/// elements of one shape, or a record of fields at their offsets. A value is built in memory of
/// one shape and read in that shape, never in another: an array of `i64` is not read as one of
/// `u8`, whose elements lie at another stride.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Mem {
    Int(IntTy),
    /// A boolean, in a byte.
    Bool,
    /// No bytes: the element of an array of no elements.
    Empty,
    /// `len` elements of one shape, each its size past the one before.
    Array {
        len: u32,
        elem: MemId,
    },
    /// Fields at their offsets within `size` bytes: a struct's, or an enum's tag at offset 0
    /// and the fields of every variant's payload, where those of two variants may overlap.
    Record {
        size: u32,
        fields: Vec<(u32, MemId)>,
    },
}

/// A shape of memory, by its place in its module's table ([`Mems`]).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct MemId(pub u32);

/// The shapes of memory a module's addresses point at, each held once: two addresses point at
/// memory of one shape exactly where their [`MemId`]s are equal. A shape is added after the
/// shapes it holds, so the table has no cycle.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Mems {
    all: Vec<Mem>,
    ids: std::collections::HashMap<Mem, MemId>,
}

impl Mems {
    /// The id of `m`, added to the table if it is not there yet.
    pub fn of(&mut self, m: Mem) -> MemId {
        if let Some(id) = self.ids.get(&m) {
            return *id;
        }
        let id = MemId(self.all.len() as u32);
        self.all.push(m.clone());
        self.ids.insert(m, id);
        id
    }

    pub fn get(&self, id: MemId) -> Option<&Mem> {
        self.all.get(id.0 as usize)
    }

    /// The bytes memory of the shape `id` takes.
    pub fn size(&self, id: MemId) -> Option<u32> {
        Some(match self.get(id)? {
            Mem::Int(t) => t.bytes(),
            Mem::Bool => 1,
            Mem::Empty => 0,
            Mem::Array { len, elem } => len.checked_mul(self.size(*elem)?)?,
            Mem::Record { size, .. } => *size,
        })
    }

    /// The type a load of memory of the shape `id` gives, and a store into it takes: an
    /// integer's or a boolean's, none for an aggregate.
    pub fn scalar(&self, id: MemId) -> Option<Ty> {
        match self.get(id)? {
            Mem::Int(t) => Some(Ty::Int(*t)),
            Mem::Bool => Some(Ty::Bool),
            Mem::Empty | Mem::Array { .. } | Mem::Record { .. } => None,
        }
    }

    /// The shape `id`, as a diagnostic names it: `[4]u8`, `{0: u8, 8: [2]i64}`.
    pub fn name(&self, id: MemId) -> String {
        match self.get(id) {
            None => format!("m{}", id.0),
            Some(Mem::Int(t)) => t.to_string(),
            Some(Mem::Bool) => "bool".into(),
            Some(Mem::Empty) => "()".into(),
            Some(Mem::Array { len, elem }) => format!("[{}]{}", len, self.name(*elem)),
            Some(Mem::Record { fields, .. }) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(at, m)| format!("{}: {}", at, self.name(*m)))
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
        }
    }
}

/// An integer whose magnitude lies below 2^128: every value of every integer type, and the
/// exact result of a shift the interpreter reports.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Num {
    pub neg: bool,
    pub mag: u128,
}

impl Num {
    pub const ZERO: Num = Num { neg: false, mag: 0 };

    pub fn new(neg: bool, mag: u128) -> Num {
        Num {
            neg: neg && mag != 0,
            mag,
        }
    }

    pub fn from_i128(v: i128) -> Num {
        Num::new(v.is_negative(), v.unsigned_abs())
    }

    fn negated(self) -> Num {
        Num::new(!self.neg, self.mag)
    }

    fn add(self, o: Num) -> Option<Num> {
        if self.neg == o.neg {
            return Some(Num::new(self.neg, self.mag.checked_add(o.mag)?));
        }
        Some(if self.mag >= o.mag {
            Num::new(self.neg, self.mag - o.mag)
        } else {
            Num::new(o.neg, o.mag - self.mag)
        })
    }

    fn mul(self, o: Num) -> Option<Num> {
        Some(Num::new(self.neg != o.neg, self.mag.checked_mul(o.mag)?))
    }

    /// The quotient truncated toward zero, and the remainder with the dividend's sign.
    fn div_rem(self, o: Num) -> (Num, Num) {
        (
            Num::new(self.neg != o.neg, self.mag / o.mag),
            Num::new(self.neg, self.mag % o.mag),
        )
    }

    /// The value's bits in `t`, which holds it.
    pub fn to_bits(self, t: IntTy) -> u128 {
        let mask = if t.bits == 128 {
            u128::MAX
        } else {
            (1 << t.bits) - 1
        };
        let bits = if self.neg {
            self.mag.wrapping_neg()
        } else {
            self.mag
        };
        bits & mask
    }

    /// The value whose bits in `t` are `bits`.
    pub fn from_bits(bits: u128, t: IntTy) -> Num {
        let mask = if t.bits == 128 {
            u128::MAX
        } else {
            (1 << t.bits) - 1
        };
        let bits = bits & mask;
        let sign = 1u128 << (t.bits - 1);
        if t.signed && bits & sign != 0 {
            Num::new(true, (!bits & mask).wrapping_add(1))
        } else {
            Num::new(false, bits)
        }
    }
}

impl Ord for Num {
    fn cmp(&self, o: &Num) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.neg, o.neg) {
            (false, false) => self.mag.cmp(&o.mag),
            (true, true) => o.mag.cmp(&self.mag),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Num {
    fn partial_cmp(&self, o: &Num) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", if self.neg { "-" } else { "" }, self.mag)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    /// Truncates toward zero.
    Div,
    /// Takes the dividend's sign.
    Rem,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum BitOp {
    And,
    Or,
    Xor,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// The integer type `t` is, if it is one.
fn int_ty(t: Ty) -> Option<IntTy> {
    match t {
        Ty::Int(i) => Some(i),
        Ty::Bool | Ty::Addr(_) | Ty::Unit | Ty::Effect => None,
    }
}

/// What a guard lets through, or an assumption states, of the value it yields ([`Op::Guard`],
/// [`Op::Assume`]): a partial operation takes the operand it is partial in from one, so that it
/// stands below what rules its bad values out by that operand alone, whatever the chain of
/// effects is rewired to ([`verify_guards`]).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Fact {
    /// The value is not zero: a divisor.
    NonZero,
    /// The value lies in `lo ..= hi`: an index, a shift amount, or a value narrowed into a type.
    Within(Num, Num),
}

impl Fact {
    /// Whether the fact holds of `n`.
    pub fn holds(self, n: Num) -> bool {
        match self {
            Fact::NonZero => n != Num::ZERO,
            Fact::Within(lo, hi) => lo <= n && n <= hi,
        }
    }

    /// Whether every value the fact holds of, `need` holds of too.
    pub fn implies(self, need: Fact) -> bool {
        match (self, need) {
            (Fact::NonZero, Fact::NonZero) => true,
            (Fact::Within(lo, hi), Fact::NonZero) => hi < Num::ZERO || Num::ZERO < lo,
            (Fact::Within(lo, hi), Fact::Within(a, b)) => a <= lo && hi <= b,
            (Fact::NonZero, Fact::Within(..)) => false,
        }
    }
}

/// What a partial operation needs of the operand it is partial in ([`Function::partial_in`]).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Need {
    /// A divisor: not zero.
    NonZero,
    /// A shift amount: within 0 to 127.
    Amount,
    /// An element's index: not negative, and within the addresses.
    Index,
    /// A value narrowed: within the type it narrows into.
    Fits(IntTy),
}

impl Need {
    /// The fact the operand must hold.
    pub fn fact(self) -> Fact {
        let top = |hi: u128| Fact::Within(Num::ZERO, Num::new(false, hi));
        match self {
            Need::NonZero => Fact::NonZero,
            Need::Amount => top(127),
            Need::Index => top(u128::from(u64::MAX)),
            Need::Fits(t) => Fact::Within(t.min(), t.max()),
        }
    }
}

impl fmt::Display for Fact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fact::NonZero => write!(f, "nonzero"),
            Fact::Within(lo, hi) => write!(f, "within {}..={}", lo, hi),
        }
    }
}

impl Op {
    /// How an instruction of this kind stands to the chain of effects ([`Chain`]). The builder
    /// threads what this names, and the verifier holds it, so that the two keep one list.
    pub fn chain(&self) -> Chain {
        match self {
            // May trap: a checked operation, a shift left, a check. Reads or writes memory: a
            // load, a store, a copy. Counts a call's frame, or makes a call.
            // An assumption, which costs nothing at run time, stands where its fact holds.
            Op::Arith { trap: Some(_), .. }
            | Op::Neg { trap: Some(_) }
            | Op::Shl { .. }
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Enter { .. }
            | Op::Call(_) => Chain::Follows,
            // Exact arithmetic and negation: exact where the static ranges of their operands
            // hold, which the verifier does not check ([`verify_guards`]).
            Op::Arith {
                op: ArithOp::Add | ArithOp::Sub | ArithOp::Mul,
                trap: None,
            }
            | Op::Neg { trap: None } => Chain::Reads,
            // A division, a shift right, a cast and an element's address are partial in one
            // operand, which they take from a guard or an assumption ([`verify_guards`]).
            Op::Arith {
                op: ArithOp::Div | ArithOp::Rem,
                trap: None,
            }
            | Op::Shr
            | Op::Cast
            | Op::Elem { .. }
            | Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Bits(_)
            | Op::Not
            | Op::Cmp(_)
            | Op::Slot { .. } => Chain::Free,
        }
    }

    /// The fact this instruction states of the value it yields, where it is a guard or an
    /// assumption, by its place in the function's table.
    pub fn fact(&self) -> Option<FactId> {
        match self {
            Op::Guard { fact, .. } | Op::Assume(fact) => Some(*fact),
            Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Shl { .. }
            | Op::Shr
            | Op::Cmp(_)
            | Op::Cast
            | Op::Check { .. }
            | Op::Slot { .. }
            | Op::Elem { .. }
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Enter { .. }
            | Op::Call(_) => None,
        }
    }

    /// The function whose frame this instruction counts, where it is a count, an [`Op::Enter`].
    pub fn counted(&self) -> Option<FuncId> {
        match self {
            Op::Enter { callee, .. } => Some(*callee),
            Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Shl { .. }
            | Op::Shr
            | Op::Cmp(_)
            | Op::Cast
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Slot { .. }
            | Op::Elem { .. }
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Call(_) => None,
        }
    }
}

/// How an instruction stands to the chain of effects, which holds the order of every effect along
/// every path: every block's last parameter, of type [`Ty::Effect`], is the latest effect where
/// the block begins; an effect takes the latest as its last operand and is the latest after it;
/// and every terminator takes the latest, a jump and a branch passing it to their targets' effect
/// parameter, a return and a trap keeping it. So each effect is taken once along every path, by
/// the next effect or by a terminator, and none is left that nothing takes.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Chain {
    /// An effect: it may trap, read or write memory, or count or make a call.
    Follows,
    /// A partial operation, defined on only some inputs: it reads the latest effect as its last
    /// operand, and does not become it, so that it stays below every check and branch that
    /// guards it while it moves freely among pure operations.
    Reads,
    /// A total operation, which takes no effect: a constant, a comparison, a bit operation, or a
    /// slot's address.
    Free,
}

/// What a value is: its operation, with what is not a value; its operands are in the pool.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Op {
    /// A block's parameter.
    Param,
    /// An integer of the value's type, as that type's bits.
    Int(u128),
    Bool(bool),
    /// `a op b`, operands and result of the value's type. Exact (`trap` none) when the static
    /// ranges guarantee the result fits, which a checker then never sees fail; checked
    /// otherwise, trapping at `trap` when it does not fit, with the exact result when its
    /// magnitude lies below 2^128 and `the result` beyond.
    Arith {
        op: ArithOp,
        trap: Option<TrapId>,
    },
    /// `-a`, exact or checked as `Arith`.
    Neg {
        trap: Option<TrapId>,
    },
    /// `a op b` on the bits of the value's type, or on booleans.
    Bits(BitOp),
    /// `~a` on the bits of the value's type, or `not` on a boolean.
    Not,
    /// `a · 2^k` in the value's type, `a` of any integer type; when it does not fit, a trap at
    /// `trap` that reports the exact value when its magnitude lies below 2^128. `k` lies within
    /// the width of the value's type.
    Shl {
        trap: TrapId,
    },
    /// `a / 2^k`, rounded toward negative infinity.
    Shr,
    /// Compares two values of one type.
    Cmp(CmpOp),
    /// The operand's value in the value's type, which holds it: an extension, or a reduction
    /// after a check.
    Cast,
    /// Traps at `trap` unless the boolean operand holds; the operands after it are the values
    /// the message reports.
    Check {
        trap: TrapId,
    },
    /// A check that yields the value it guards, its last operand: traps at `trap` unless the
    /// boolean operand holds, reporting the operands between, and yields the value, of which
    /// `fact` then holds. A partial operation takes what it yields ([`Fact`]).
    Guard {
        trap: TrapId,
        fact: FactId,
    },
    /// Yields its operand, of which `fact` holds where it stands: a guard Tier 1 proved, left out,
    /// or a fact a type gives, which costs nothing at run time (the Ghost Contract `Assume` of
    /// Engineering Guardrail #2). The MIR interpreter reports broken MIR where it does not hold.
    Assume(FactId),
    /// A new stack slot of `bytes`, its address the value, at the alignment its size implies,
    /// the largest power of two up to 16 that divides it.
    Slot {
        bytes: u32,
    },
    /// The address `base + index · stride`; the index lies within its array.
    Elem {
        stride: u32,
    },
    /// The value of the value's type at the address.
    Load,
    /// Stores the second operand at the first, an address.
    Store,
    /// Copies `bytes` from the second address to the first.
    Copy {
        bytes: u32,
    },
    /// Calls a function of the module on its operands, the latest effect last, which fills the
    /// callee's effect parameter ([`Chain`]); the value is its result, or unit, and it is the
    /// latest effect after it. The [`Op::Enter`] before it counted the callee's frame, and the call gives those
    /// bytes back when it returns.
    Call(FuncId),
    /// Counts the frame of `callee` against the run's stack budget, where the call is counted:
    /// its arguments evaluated, none narrowed yet, the callee's checks not begun
    /// (`core-calculus.md`, item 25). Its operands are those of its call, the arguments as
    /// evaluated, before any is narrowed, and the latest effect last; it is the latest effect
    /// after it, which the callee's checks take ([`Chain`]). So the chain holds its place: the
    /// count stands after its arguments and the effects before it, and before every narrowing,
    /// check and call after it. Traps at `trap` where the stack in use and
    /// that frame together exceed the budget; otherwise the bytes are in use until the call
    /// returns. Every path from an `Enter` reaches its call or a trap, and the calls that a
    /// callee's checks make between them are each counted and returned within. The lowering
    /// always gives it its trap; one without (`trap` none) is exact, which the mutation gate
    /// makes, and the MIR interpreter reports broken MIR where it would run out.
    Enter {
        callee: FuncId,
        trap: Option<TrapId>,
    },
}

/// How a block ends.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Term {
    Jump(BlockId, Vec<NodeId>),
    /// To the first block when the condition holds, else to the second.
    Branch {
        cond: NodeId,
        then: (BlockId, Vec<NodeId>),
        otherwise: (BlockId, Vec<NodeId>),
    },
    /// Returns the value, if any, after the latest effect, which it takes.
    Return(Option<NodeId>, NodeId),
    /// Traps at the site after the latest effect, which it takes.
    Trap(TrapId, NodeId),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Block {
    pub params: Vec<NodeId>,
    pub insts: Vec<NodeId>,
    pub term: Term,
}

/// A place in the source: a line and a column, both from 1.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
}

/// Where a trap is raised and what it reports, as the reference interpreter prints it:
/// `trap[kind] at line:col: ` and then the pieces of `text` with the values the trap reports
/// between them, so `text` has one piece more than there are values. A value the trap cannot
/// know, the exact result of an operation beyond 128 bits, reads `the result`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrapSite {
    pub kind: String,
    pub loc: Loc,
    pub text: Vec<String>,
}

/// One function: a row per value, structure of arrays, and its blocks.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub result: Option<Ty>,
    /// The bytes a call of this function counts against the stack budget, the frame the checker
    /// computed for it (`core-calculus.md`, item 25); 0 for an entry, which no program calls.
    pub frame: u128,
    pub ops: Vec<Op>,
    pub tys: Vec<Ty>,
    /// Each value's operands, a range of `pool`.
    pub operands: Vec<(u32, u32)>,
    pub locs: Vec<Loc>,
    pub pool: Vec<NodeId>,
    pub blocks: Vec<Block>,
    pub traps: Vec<TrapSite>,
    /// The facts its guards and assumptions state.
    pub facts: Vec<Fact>,
}

impl Function {
    pub fn args(&self, v: NodeId) -> &[NodeId] {
        let (start, len) = self.operands[v.0 as usize];
        &self.pool[start as usize..(start + len) as usize]
    }

    pub fn ty(&self, v: NodeId) -> Ty {
        self.tys[v.0 as usize]
    }

    /// The effect instruction `v` takes, as its last operand, where its operation follows or reads
    /// the latest effect ([`Op::chain`]): the verifier holds it to the latest effect where `v`
    /// stands ([`Chain`]).
    pub fn effect(&self, v: NodeId) -> Option<NodeId> {
        match self.op(v).chain() {
            Chain::Follows | Chain::Reads => self.args(v).last().copied(),
            Chain::Free => None,
        }
    }

    /// The fact that holds of value `v` wherever it is defined: a guard's or an assumption's, of
    /// what it yields; a constant's, its own value; and a cast's, its operand's, whose value it
    /// keeps where it verifies. None for any other value.
    pub fn fact(&self, v: NodeId) -> Option<Fact> {
        let op = self.ops.get(v.0 as usize)?;
        match op {
            Op::Int(bits) => int_ty(self.ty(v)).map(|t| {
                let n = Num::from_bits(*bits, t);
                Fact::Within(n, n)
            }),
            Op::Cast => self.own_args(v).first().and_then(|a| self.fact(*a)),
            Op::Param
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Shl { .. }
            | Op::Shr
            | Op::Cmp(_)
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Slot { .. }
            | Op::Elem { .. }
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Enter { .. }
            | Op::Call(_) => op
                .fact()
                .and_then(|id| self.facts.get(id.0 as usize).copied()),
        }
    }

    /// The operand instruction `v` is partial in, by its place among its own operands, and what
    /// it needs of it: a division's divisor, not zero; a shift's amount, within 0 to 127; an
    /// element's index, not negative; and a narrowing cast's operand, within the type it
    /// narrows into. None for an instruction defined on every operand.
    pub fn partial_in(&self, v: NodeId) -> Option<(usize, Need)> {
        match self.op(v) {
            Op::Arith {
                op: ArithOp::Div | ArithOp::Rem,
                ..
            } => Some((1, Need::NonZero)),
            Op::Shl { .. } | Op::Shr => Some((1, Need::Amount)),
            Op::Elem { .. } => Some((1, Need::Index)),
            Op::Cast => {
                let from = self.own_args(v).first().and_then(|a| int_ty(self.ty(*a)));
                from.zip(int_ty(self.ty(v)))
                    .filter(|(from, to)| !(to.holds(from.min()) && to.holds(from.max())))
                    .map(|(_, to)| (0, Need::Fits(to)))
            }
            Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Cmp(_)
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Slot { .. }
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Enter { .. }
            | Op::Call(_) => None,
        }
    }

    /// A fact of the table as the printed form writes it.
    fn fact_text(&self, id: FactId) -> String {
        self.facts
            .get(id.0 as usize)
            .map_or(format!("f{}", id.0), |f| f.to_string())
    }

    /// The operands of `v` besides the effect it takes: of a call, the callee's arguments.
    pub fn own_args(&self, v: NodeId) -> &[NodeId] {
        let args = self.args(v);
        match self.op(v).chain() {
            Chain::Follows | Chain::Reads => &args[..args.len().saturating_sub(1)],
            Chain::Free => args,
        }
    }

    pub fn op(&self, v: NodeId) -> &Op {
        &self.ops[v.0 as usize]
    }

    pub fn block(&self, b: BlockId) -> &Block {
        &self.blocks[b.0 as usize]
    }

    /// An instruction's operation as the printed form writes it: `add`, `add t2` for a
    /// checked one, `int -5`, `check t0`, `slot 16`, `call f1`.
    fn op_text(&self, v: NodeId) -> String {
        let trapped = |name: &str, t: &Option<TrapId>| match t {
            Some(t) => format!("{} t{}", name, t.0),
            None => name.to_string(),
        };
        match self.op(v) {
            Op::Param => "param".into(),
            Op::Int(bits) => match self.ty(v) {
                Ty::Int(t) => format!("int {}", Num::from_bits(*bits, t)),
                Ty::Bool | Ty::Addr(_) | Ty::Unit | Ty::Effect => format!("int {}", bits),
            },
            Op::Bool(b) => format!("bool {}", b),
            Op::Arith { op, trap } => {
                let name = match op {
                    ArithOp::Add => "add",
                    ArithOp::Sub => "sub",
                    ArithOp::Mul => "mul",
                    ArithOp::Div => "div",
                    ArithOp::Rem => "rem",
                };
                trapped(name, trap)
            }
            Op::Neg { trap } => trapped("neg", trap),
            Op::Bits(BitOp::And) => "and".into(),
            Op::Bits(BitOp::Or) => "or".into(),
            Op::Bits(BitOp::Xor) => "xor".into(),
            Op::Not => "not".into(),
            Op::Shl { trap } => format!("shl t{}", trap.0),
            Op::Shr => "shr".into(),
            Op::Cmp(op) => match op {
                CmpOp::Eq => "eq",
                CmpOp::Ne => "ne",
                CmpOp::Lt => "lt",
                CmpOp::Le => "le",
                CmpOp::Gt => "gt",
                CmpOp::Ge => "ge",
            }
            .into(),
            Op::Cast => "cast".into(),
            Op::Check { trap } => format!("check t{}", trap.0),
            Op::Guard { trap, fact } => format!("guard t{} {}", trap.0, self.fact_text(*fact)),
            Op::Assume(fact) => format!("assume {}", self.fact_text(*fact)),
            Op::Slot { bytes } => format!("slot {}", bytes),
            Op::Elem { stride } => format!("elem {}", stride),
            Op::Load => "load".into(),
            Op::Store => "store".into(),
            Op::Copy { bytes } => format!("copy {}", bytes),
            Op::Call(c) => format!("call f{}", c.0),
            Op::Enter { callee, trap } => trapped(&format!("enter f{}", callee.0), trap),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Module {
    pub functions: Vec<Function>,
    /// The run's stack budget in bytes, which the lowering takes from the one the checker
    /// defines (`core-calculus.md`, item 25): every [`Op::Enter`] counts against it.
    pub budget: u128,
    /// The shapes of memory its addresses point at ([`Ty::Addr`]).
    pub mems: Mems,
}

impl Module {
    /// The frame of `callee`: the bytes a call of it counts against the budget.
    pub fn frame_of(&self, callee: FuncId) -> u128 {
        self.functions.get(callee.0 as usize).map_or(0, |c| c.frame)
    }

    /// Whether a call of `callee`, made with `stack` bytes of frames in use, runs out of the
    /// budget: the stack in use and the callee's frame together exceed it, and a call that fills
    /// it exactly runs (`core-calculus.md`, item 25).
    pub fn runs_out(&self, stack: u128, callee: FuncId) -> bool {
        stack + self.frame_of(callee) > self.budget
    }
}

/// Builds a function one block at a time.
pub struct Builder {
    f: Function,
    current: BlockId,
    loc: Loc,
    /// The latest effect of each block: its effect parameter, until an effect follows it.
    latest: Vec<NodeId>,
}

impl Builder {
    /// A function whose entry block takes `params`.
    pub fn new(name: &str, params: &[Ty], result: Option<Ty>) -> Builder {
        Builder::declared_at(name, params, result, Loc::default())
    }

    /// A function whose entry block takes `params`, declared at `decl`: its parameters carry
    /// that location, which a debugger names as the function's. Its last parameter is the latest
    /// effect before the call ([`Chain`]).
    pub fn declared_at(name: &str, params: &[Ty], result: Option<Ty>, decl: Loc) -> Builder {
        let mut with = params.to_vec();
        with.push(Ty::Effect);
        let mut b = Builder {
            f: Function {
                name: name.to_string(),
                params: with,
                result,
                ..Function::default()
            },
            current: BlockId(0),
            loc: decl,
            latest: Vec::new(),
        };
        let entry = b.block(params);
        b.switch(entry);
        b.loc = Loc::default();
        b
    }

    /// The location of the instructions built from now on.
    pub fn at(&mut self, line: u32, col: u32) {
        self.loc = Loc { line, col };
    }

    fn value(&mut self, op: Op, ty: Ty, args: &[NodeId]) -> NodeId {
        let v = NodeId(self.f.ops.len() as u32);
        self.f.ops.push(op);
        self.f.tys.push(ty);
        self.f
            .operands
            .push((self.f.pool.len() as u32, args.len() as u32));
        self.f.pool.extend_from_slice(args);
        self.f.locs.push(self.loc);
        v
    }

    /// A new block taking `params` and, last, the latest effect where it begins; it returns
    /// until it is given its terminator.
    pub fn block(&mut self, params: &[Ty]) -> BlockId {
        let id = BlockId(self.f.blocks.len() as u32);
        let mut taken: Vec<NodeId> = params
            .iter()
            .map(|t| self.value(Op::Param, *t, &[]))
            .collect();
        let effect = self.value(Op::Param, Ty::Effect, &[]);
        taken.push(effect);
        self.f.blocks.push(Block {
            params: taken,
            insts: Vec::new(),
            term: Term::Return(None, effect),
        });
        self.latest.push(effect);
        id
    }

    /// The parameters of block `b` besides its effect parameter.
    pub fn params(&self, b: BlockId) -> Vec<NodeId> {
        let params = &self.f.block(b).params;
        params[..params.len().saturating_sub(1)].to_vec()
    }

    /// The fact that holds of value `v` wherever it is defined ([`Function::fact`]).
    pub fn fact(&self, v: NodeId) -> Option<Fact> {
        self.f.fact(v)
    }

    /// The latest effect of the current block.
    pub fn latest(&self) -> NodeId {
        self.latest[self.current.0 as usize]
    }

    /// Builds into block `b` from now on.
    pub fn switch(&mut self, b: BlockId) {
        self.current = b;
    }

    pub fn current(&self) -> BlockId {
        self.current
    }

    /// Appends an instruction to the current block, taking `args` and, where its operation
    /// follows or reads the latest effect, that effect last ([`Op::chain`]).
    pub fn inst(&mut self, op: Op, ty: Ty, args: &[NodeId]) -> NodeId {
        let role = op.chain();
        let latest = self.latest();
        let v = match role {
            Chain::Follows | Chain::Reads => {
                let mut with = args.to_vec();
                with.push(latest);
                self.value(op, ty, &with)
            }
            Chain::Free => self.value(op, ty, args),
        };
        self.f.blocks[self.current.0 as usize].insts.push(v);
        match role {
            Chain::Follows => self.latest[self.current.0 as usize] = v,
            Chain::Reads | Chain::Free => {}
        }
        v
    }

    pub fn int(&mut self, t: IntTy, n: Num) -> NodeId {
        self.inst(Op::Int(n.to_bits(t)), Ty::Int(t), &[])
    }

    pub fn trap_site(&mut self, kind: &str, text: &[&str]) -> TrapId {
        let id = TrapId(self.f.traps.len() as u32);
        self.f.traps.push(TrapSite {
            kind: kind.to_string(),
            loc: self.loc,
            text: text.iter().map(|s| s.to_string()).collect(),
        });
        id
    }

    fn end(&mut self, term: Term) {
        self.f.blocks[self.current.0 as usize].term = term;
    }

    /// A fact of the function's table, for a guard or an assumption to state: one the table holds
    /// already is given again.
    pub fn fact_site(&mut self, fact: Fact) -> FactId {
        match self.f.facts.iter().position(|f| *f == fact) {
            Some(i) => FactId(i as u32),
            None => {
                self.f.facts.push(fact);
                FactId(self.f.facts.len() as u32 - 1)
            }
        }
    }

    /// The value `v`, of which `fact` holds where it stands ([`Op::Assume`]).
    pub fn assume(&mut self, v: NodeId, fact: Fact) -> NodeId {
        let ty = self.f.ty(v);
        let fact = self.fact_site(fact);
        self.inst(Op::Assume(fact), ty, &[v])
    }

    /// The value last among `args` let through where the condition first among them holds, of
    /// which `fact` then holds; elsewhere it traps at `trap`, reporting those between
    /// ([`Op::Guard`]).
    pub fn guard(&mut self, trap: TrapId, fact: Fact, args: &[NodeId]) -> NodeId {
        let ty = args.last().map_or(Ty::Unit, |v| self.f.ty(*v));
        let fact = self.fact_site(fact);
        self.inst(Op::Guard { trap, fact }, ty, args)
    }

    /// Ends the current block in a jump to `to` with `args` and the latest effect.
    pub fn jump(&mut self, to: BlockId, mut args: Vec<NodeId>) {
        args.push(self.latest());
        self.end(Term::Jump(to, args));
    }

    /// Ends the current block in a branch on `cond`, each target given its arguments and the
    /// latest effect.
    pub fn branch(
        &mut self,
        cond: NodeId,
        mut then: (BlockId, Vec<NodeId>),
        mut otherwise: (BlockId, Vec<NodeId>),
    ) {
        let latest = self.latest();
        then.1.push(latest);
        otherwise.1.push(latest);
        self.end(Term::Branch {
            cond,
            then,
            otherwise,
        });
    }

    /// Ends the current block in a return of `value`, if any, after the latest effect.
    pub fn ret(&mut self, value: Option<NodeId>) {
        let latest = self.latest();
        self.end(Term::Return(value, latest));
    }

    /// Ends the current block in a trap at `site`, after the latest effect.
    pub fn trap(&mut self, site: TrapId) {
        let latest = self.latest();
        self.end(Term::Trap(site, latest));
    }

    pub fn finish(self) -> Function {
        self.f
    }
}

// -------------------------------------------------------------------------------------------
// Verification
// -------------------------------------------------------------------------------------------

/// The ways a function can be malformed, each found by [`verify`].
pub fn verify(m: &Module) -> Result<(), String> {
    verify_mems(&m.mems)?;
    for f in &m.functions {
        verify_function(m, f).map_err(|e| format!("fn {}: {}", f.name, e))?;
    }
    Ok(())
}

/// Every shape of memory holds only shapes added before it, so the table has no cycle; an
/// array's bytes fit 32 bits; and a record's fields lie within its bytes.
fn verify_mems(mems: &Mems) -> Result<(), String> {
    for (i, m) in mems.all.iter().enumerate() {
        let id = MemId(i as u32);
        let before = |c: &MemId| c.0 < id.0;
        let fits = match m {
            Mem::Int(_) | Mem::Bool | Mem::Empty => true,
            Mem::Array { elem, .. } => before(elem) && mems.size(id).is_some(),
            Mem::Record { size, fields } => fields.iter().all(|(at, f)| {
                before(f)
                    && mems
                        .size(*f)
                        .and_then(|s| at.checked_add(s))
                        .is_some_and(|end| end <= *size)
            }),
        };
        if !fits {
            return Err(format!(
                "memory m{} holds a shape not added before it, or more bytes than it has",
                id.0
            ));
        }
    }
    Ok(())
}

fn verify_function(m: &Module, f: &Function) -> Result<(), String> {
    let n = f.ops.len();
    let entry = f.blocks.first().ok_or("no entry block")?;
    let entry_tys: Vec<Ty> = entry.params.iter().map(|p| f.ty(*p)).collect();
    if entry_tys != f.params {
        return Err("the entry block's parameters are not the function's".into());
    }
    // Every address points at memory of a shape the module holds.
    if let Some(v) = (0..n).find(|&v| match f.tys[v] {
        Ty::Addr(id) => m.mems.get(id).is_none(),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Effect => false,
    }) {
        return Err(format!(
            "v{} is the address of memory of a shape the module does not hold",
            v
        ));
    }
    // No function returns an address: a slot's address never outlives the call that made it.
    if f.result.is_some_and(|t| t.mem().is_some()) {
        return Err("the function returns an address, which would outlive its slot".into());
    }
    let defined = |v: NodeId| (v.0 as usize) < n;
    let jump = |(b, args): &(BlockId, Vec<NodeId>)| -> Result<(), String> {
        let target = f
            .blocks
            .get(b.0 as usize)
            .ok_or(format!("a jump to block {} that does not exist", b.0))?;
        // Where branches meet, an array built in one shape is never taken in another.
        for (a, p) in args
            .iter()
            .zip(&target.params)
            .filter(|(a, _)| defined(**a))
        {
            match (f.ty(*a).mem(), f.ty(*p).mem()) {
                (Some(given), Some(taken)) if given != taken => {
                    return Err(format!(
                        "a jump to block {} passes memory of {} where it takes memory of {}",
                        b.0,
                        m.mems.name(given),
                        m.mems.name(taken)
                    ));
                }
                (Some(_) | None, Some(_) | None) => {}
            }
        }
        let admitted = args.len() == target.params.len()
            && args
                .iter()
                .zip(&target.params)
                .all(|(a, p)| Want::param(f.ty(*p)).admits(f.ty(*a)));
        if args.iter().any(|a| !defined(*a)) || !admitted {
            return Err(format!("a jump to block {} with the wrong arguments", b.0));
        }
        Ok(())
    };
    for (bi, b) in f.blocks.iter().enumerate() {
        // The block's last parameter, and only it, is the latest effect where it begins.
        let effects: Vec<bool> = b.params.iter().map(|p| f.ty(*p) == Ty::Effect).collect();
        if effects.last() != Some(&true) || effects.iter().filter(|e| **e).count() != 1 {
            return Err(format!(
                "block {} does not take the latest effect as its last parameter, and only there",
                bi
            ));
        }
        for &v in &b.insts {
            let at = || format!("v{} in block {}", v.0, bi);
            if !defined(v) || f.args(v).iter().any(|a| !defined(*a)) {
                return Err(format!("{}: an operand that does not exist", at()));
            }
            check_inst(m, f, v).map_err(|e| format!("{}: {}", at(), e))?;
            check_memory(&m.mems, f, v).map_err(|e| format!("{}: {}", at(), e))?;
        }
        match &b.term {
            Term::Jump(t, args) => jump(&(*t, args.clone()))?,
            Term::Branch {
                cond,
                then,
                otherwise,
            } => {
                if !defined(*cond) || f.ty(*cond) != Ty::Bool {
                    return Err(format!(
                        "block {} branches on a value that is not a bool",
                        bi
                    ));
                }
                jump(then)?;
                jump(otherwise)?;
            }
            Term::Return(v, effect) => {
                let given = v.filter(|v| defined(*v)).map(|v| f.ty(v));
                if given != f.result || !defined(*effect) {
                    return Err(format!("block {} returns the wrong type", bi));
                }
            }
            Term::Trap(t, effect) => {
                if f.traps.get(t.0 as usize).is_none_or(|s| s.text.len() != 1) || !defined(*effect)
                {
                    return Err(format!("block {} traps at a site that does not exist", bi));
                }
            }
        }
    }
    verify_order(f)?;
    verify_chain(f)?;
    verify_guards(f)?;
    verify_frames(f)
}

/// Every partial operation takes the operand it is partial in from a guard or an assumption whose
/// fact rules its bad values out, or as a constant that is not one ([`Function::partial_in`],
/// [`Function::fact`]): a division's divisor from one that holds it is not zero, a shift's amount
/// from one that holds it within 0 to 127, an element's index from one that holds it is not
/// negative, and a narrowing cast's operand from one that holds it within the type it narrows
/// into. A partial operation then stands below its guard by that operand alone, which is not
/// defined above the guard, whatever the chain of effects is rewired to. Exact arithmetic and
/// negation are exact where the static ranges of their operands hold, which come from types and
/// refinements this does not see; they read the latest effect instead ([`Chain::Reads`]).
#[inline(never)]
fn verify_guards(f: &Function) -> Result<(), String> {
    for (bi, b) in f.blocks.iter().enumerate() {
        for &v in &b.insts {
            let Some((k, need)) = f.partial_in(v) else {
                continue;
            };
            let Some(&a) = f.own_args(v).get(k) else {
                continue;
            };
            if !f.fact(a).is_some_and(|fact| fact.implies(need.fact())) {
                return Err(format!(
                    "v{} in block {}: it takes v{}, of which no guard or assumption holds {}",
                    v.0,
                    bi,
                    a.0,
                    need.fact()
                ));
            }
        }
    }
    Ok(())
}

/// The effects of `f` form one chain along every path ([`Chain`]): in each block, from its
/// effect parameter, every effect takes the latest effect and is the latest after it, every
/// partial operation reads the latest, and the terminator takes the latest, a jump or a branch
/// passing it to its targets' effect parameter. So no effect is taken twice along a path, which
/// would fork the chain, none is left that nothing takes, which a pass that keeps only what is
/// used would delete, and no effect or partial operation moves past another effect while it
/// takes the effect it took: an instruction moved keeps its operands, which are no longer the
/// latest where it lands.
fn verify_chain(f: &Function) -> Result<(), String> {
    let name = |e: Option<NodeId>| e.map_or("no effect".to_string(), |e| format!("v{}", e.0));
    for (bi, b) in f.blocks.iter().enumerate() {
        let Some(&first) = b.params.last() else {
            return Err(format!("block {} takes no effect", bi));
        };
        let mut latest = first;
        for &v in &b.insts {
            let role = f.op(v).chain();
            let taken = match role {
                Chain::Follows | Chain::Reads => f.args(v).last().copied(),
                Chain::Free => continue,
            };
            if taken != Some(latest) {
                return Err(format!(
                    "v{} in block {}: it takes {} where the latest effect is v{}",
                    v.0,
                    bi,
                    name(taken),
                    latest.0
                ));
            }
            match role {
                Chain::Follows => latest = v,
                Chain::Reads | Chain::Free => {}
            }
        }
        let taken: Vec<Option<NodeId>> = match &b.term {
            Term::Jump(_, args) => vec![args.last().copied()],
            Term::Branch {
                then, otherwise, ..
            } => vec![then.1.last().copied(), otherwise.1.last().copied()],
            Term::Return(_, effect) | Term::Trap(_, effect) => vec![Some(*effect)],
        };
        if let Some(wrong) = taken.into_iter().find(|t| *t != Some(latest)) {
            return Err(format!(
                "block {}: its terminator takes {} where the latest effect is v{}",
                bi,
                name(wrong),
                latest.0
            ));
        }
    }
    Ok(())
}

/// The blocks each block's terminator may go to.
fn successors(term: &Term) -> Vec<BlockId> {
    match term {
        Term::Jump(t, _) => vec![*t],
        Term::Branch {
            then, otherwise, ..
        } => vec![then.0, otherwise.0],
        Term::Return(..) | Term::Trap(..) => Vec::new(),
    }
}

/// The dominator tree of a function's blocks reached from its entry: each block's immediate
/// dominator, by the iterative algorithm of Cooper, Harvey and Kennedy over reverse postorder, and
/// the blocks each block dominates as a range of a walk of the tree, so that whether one block
/// dominates another is read in constant time.
struct Dominators {
    /// The places, in the order a walk of the tree enters the blocks, of each block and of the
    /// blocks it dominates; none for a block no path reaches.
    span: Vec<Option<Range<usize>>>,
}

impl Dominators {
    /// The dominators of `f`, which has an entry block and jumps only to its own blocks: the
    /// verifier checks both before it reads the order of values (`verify_function`).
    fn of(f: &Function) -> Dominators {
        let n = f.blocks.len();
        let next: Vec<Vec<usize>> = f
            .blocks
            .iter()
            .map(|b| successors(&b.term).iter().map(|t| t.0 as usize).collect())
            .collect();
        // Reverse postorder of the blocks reached, by a walk that keeps its own stack.
        let mut order = Vec::new();
        let mut seen = vec![false; n];
        seen[0] = true;
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        while let Some((b, k)) = stack.pop() {
            match next[b].get(k) {
                Some(&t) => {
                    stack.push((b, k + 1));
                    if !seen[t] {
                        seen[t] = true;
                        stack.push((t, 0));
                    }
                }
                None => order.push(b),
            }
        }
        order.reverse();
        let mut rank = vec![usize::MAX; n];
        for (i, &b) in order.iter().enumerate() {
            rank[b] = i;
        }
        let mut before: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &b in &order {
            for &t in &next[b] {
                before[t].push(b);
            }
        }
        let mut idom = vec![usize::MAX; n];
        idom[0] = 0;
        let meet = |idom: &[usize], mut a: usize, mut b: usize| {
            while a != b {
                while rank[a] > rank[b] {
                    a = idom[a];
                }
                while rank[b] > rank[a] {
                    b = idom[b];
                }
            }
            a
        };
        let mut changed = true;
        while changed {
            changed = false;
            for &b in order.iter().skip(1) {
                let mut new = usize::MAX;
                for &p in &before[b] {
                    if idom[p] != usize::MAX {
                        new = if new == usize::MAX {
                            p
                        } else {
                            meet(&idom, p, new)
                        };
                    }
                }
                if new != idom[b] {
                    idom[b] = new;
                    changed = true;
                }
            }
        }
        // A walk of the tree, each block's children after it.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &b in order.iter().skip(1) {
            children[idom[b]].push(b);
        }
        // A block is entered before the blocks it dominates and left after them, so they are
        // the blocks entered from its own entry until it is left.
        let mut span = vec![None; n];
        let mut entered = 0;
        let mut walk: Vec<(usize, Option<usize>)> = vec![(0, None)];
        while let Some((b, start)) = walk.pop() {
            match start {
                Some(start) => span[b] = Some(start..entered),
                None => {
                    walk.push((b, Some(entered)));
                    entered += 1;
                    walk.extend(children[b].iter().map(|c| (*c, None)));
                }
            }
        }
        Dominators { span }
    }

    /// Whether a path from the entry reaches block `b`.
    fn reached(&self, b: usize) -> bool {
        self.span.get(b).is_some_and(|s| s.is_some())
    }

    /// Whether block `a` dominates block `b`, both reached: every path from the entry to `b`
    /// passes through `a`.
    fn dominates(&self, a: usize, b: usize) -> bool {
        match (
            self.span.get(a).cloned().flatten(),
            self.span.get(b).cloned().flatten(),
        ) {
            (Some(a), Some(b)) => a.contains(&b.start),
            (Some(_), None) | (None, Some(_)) | (None, None) => false,
        }
    }
}

/// Every value is defined before it is used, on every path: an operand of an instruction is a
/// parameter of its block, an instruction before it in its block, or a value of a block that
/// dominates its block, and an operand of a terminator likewise. So nothing reads a value before
/// an instruction gives it, and the order of a call's count is held by its values: a count
/// stands after the arguments it takes, and before the narrowings, checks and call that take it
/// ([`Op::Enter`]). A block no path reaches is not checked.
fn verify_order(f: &Function) -> Result<(), String> {
    // Where each value is defined: its block, and its place there, 0 for a parameter.
    let mut defined: Vec<Option<(usize, usize)>> = vec![None; f.ops.len()];
    for (bi, b) in f.blocks.iter().enumerate() {
        for &p in &b.params {
            defined[p.0 as usize] = Some((bi, 0));
        }
        for (i, &v) in b.insts.iter().enumerate() {
            defined[v.0 as usize] = Some((bi, i + 1));
        }
    }
    let dom = Dominators::of(f);
    let reached = |b: usize| dom.reached(b);
    let available = |a: NodeId, b: usize, place: usize| match defined.get(a.0 as usize) {
        Some(Some((ab, ap))) if *ab == b => *ap < place,
        Some(Some((ab, _))) => dom.dominates(*ab, b),
        _ => false,
    };
    for (b, block) in f.blocks.iter().enumerate() {
        if !reached(b) {
            continue;
        }
        for (i, &v) in block.insts.iter().enumerate() {
            if let Some(a) = f.args(v).iter().find(|a| !available(**a, b, i + 1)) {
                return Err(format!(
                    "v{} in block {}: its operand v{} is not defined before it on every path",
                    v.0, b, a.0
                ));
            }
        }
        let end = block.insts.len() + 1;
        let used: Vec<NodeId> = match &block.term {
            Term::Jump(_, args) => args.clone(),
            Term::Branch {
                cond,
                then,
                otherwise,
            } => std::iter::once(*cond)
                .chain(then.1.iter().copied())
                .chain(otherwise.1.iter().copied())
                .collect(),
            Term::Return(v, effect) => v.iter().copied().chain([*effect]).collect(),
            Term::Trap(_, effect) => vec![*effect],
        };
        if let Some(a) = used.iter().find(|a| !available(**a, b, end)) {
            return Err(format!(
                "block {}: its terminator's operand v{} is not defined before it on every path",
                b, a.0
            ));
        }
    }
    Ok(())
}

/// Every call's frame is counted by an [`Op::Enter`] of its callee and given back by that call
/// alone: along every path, a call closes the latest count not yet closed, which is the count it
/// takes, so the counts of calls a callee's checks make nest within it; every path into a block
/// arrives with the same counts open; and a function returns with none open. So no call goes
/// uncounted, and no count can fall through to a return, or around a loop, without its call,
/// which would leave its bytes in use (`core-calculus.md`, item 25).
fn verify_frames(f: &Function) -> Result<(), String> {
    let mut open_at: Vec<Option<Vec<NodeId>>> = vec![None; f.blocks.len()];
    open_at[0] = Some(Vec::new());
    let mut work = vec![BlockId(0)];
    while let Some(b) = work.pop() {
        let block = &f.blocks[b.0 as usize];
        let mut open = open_at[b.0 as usize].clone().unwrap_or_default();
        for &v in &block.insts {
            match f.op(v) {
                Op::Enter { .. } => open.push(v),
                Op::Call(c) => {
                    let latest = open.pop().and_then(|count| f.op(count).counted());
                    if latest != Some(*c) {
                        return Err(format!(
                            "v{} in block {}: a call of f{} that does not close the latest count open",
                            v.0, b.0, c.0
                        ));
                    }
                }
                Op::Param
                | Op::Int(_)
                | Op::Bool(_)
                | Op::Arith { .. }
                | Op::Neg { .. }
                | Op::Bits(_)
                | Op::Not
                | Op::Shl { .. }
                | Op::Shr
                | Op::Cmp(_)
                | Op::Cast
                | Op::Check { .. }
                | Op::Guard { .. }
                | Op::Assume(_)
                | Op::Slot { .. }
                | Op::Elem { .. }
                | Op::Load
                | Op::Store
                | Op::Copy { .. } => {}
            }
        }
        let targets: Vec<BlockId> = match &block.term {
            Term::Jump(t, _) => vec![*t],
            Term::Branch {
                then, otherwise, ..
            } => vec![then.0, otherwise.0],
            Term::Return(..) => {
                if let Some(c) = open.last() {
                    return Err(format!(
                        "block {} returns with the count v{} open",
                        b.0, c.0
                    ));
                }
                Vec::new()
            }
            Term::Trap(..) => Vec::new(),
        };
        for t in targets {
            match &open_at[t.0 as usize] {
                None => {
                    open_at[t.0 as usize] = Some(open.clone());
                    work.push(t);
                }
                Some(seen) if *seen == open => {}
                Some(_) => {
                    return Err(format!(
                        "block {} is entered with different counts of calls open",
                        t.0
                    ))
                }
            }
        }
    }
    Ok(())
}

/// A type an operation's result or operand must have.
#[derive(Clone, Copy)]
enum Want {
    Is(Ty),
    Int,
    IntOrBool,
    /// An address, of memory of any shape, which [`check_memory`] holds to its access.
    Addr,
    /// A value of any type but unit: an argument as evaluated.
    Value,
    /// Any value: what a parameter of type [`Ty::Effect`] admits.
    Any,
}

impl Want {
    fn admits(self, t: Ty) -> bool {
        match self {
            Want::Is(x) => x == t,
            Want::Int => matches!(t, Ty::Int(_)),
            Want::IntOrBool => matches!(t, Ty::Int(_) | Ty::Bool),
            Want::Addr => t.mem().is_some(),
            Want::Value => t != Ty::Unit,
            Want::Any => true,
        }
    }

    /// What a parameter of type `t` admits: a value of its type, or any value for an effect.
    fn param(t: Ty) -> Want {
        match t {
            Ty::Effect => Want::Any,
            Ty::Int(_) | Ty::Bool | Ty::Addr(_) | Ty::Unit => Want::Is(t),
        }
    }
}

/// Checks an instruction against what its operation takes and gives: its result's type, its
/// operands' types, and the trap site it may raise, with room for the values it reports.
fn check_inst(m: &Module, f: &Function, v: NodeId) -> Result<(), String> {
    let ty = f.ty(v);
    let args: Vec<Ty> = f.own_args(v).iter().map(|a| f.ty(*a)).collect();
    let same = Want::Is(ty);
    let reported = |t: &Option<TrapId>| t.map(|t| (t, 1));
    let (result, operands, trap): (Want, Vec<Want>, Option<(TrapId, usize)>) = match f.op(v) {
        Op::Param => return Err("a parameter among a block's instructions".into()),
        Op::Int(_) => (Want::Int, vec![], None),
        Op::Bool(_) => (Want::Is(Ty::Bool), vec![], None),
        // A checked operation computes its exact result before it checks it, so its operands
        // may keep their own types: an `i32` times a `u128`, checked against `u128`.
        Op::Arith { trap: Some(t), .. } => {
            (Want::Int, vec![Want::Int, Want::Int], reported(&Some(*t)))
        }
        Op::Arith { trap: None, .. } => (Want::Int, vec![same, same], None),
        Op::Neg { trap } => (Want::Int, vec![same], reported(trap)),
        Op::Bits(_) => (Want::IntOrBool, vec![same, same], None),
        Op::Not => (Want::IntOrBool, vec![same], None),
        Op::Shl { trap } => (Want::Int, vec![Want::Int, Want::Int], Some((*trap, 1))),
        Op::Shr => (Want::Int, vec![same, Want::Int], None),
        Op::Cmp(_) => {
            let first = args.first().copied().unwrap_or(Ty::Unit);
            (
                Want::Is(Ty::Bool),
                vec![Want::IntOrBool, Want::Is(first)],
                None,
            )
        }
        Op::Cast => (Want::Int, vec![Want::Int], None),
        Op::Check { trap } => {
            let values = args.len().saturating_sub(1);
            let mut wants = vec![Want::Is(Ty::Bool)];
            wants.extend(std::iter::repeat_n(Want::Int, values));
            (Want::Is(Ty::Unit), wants, Some((*trap, values)))
        }
        // The condition, the values the message reports, and the integer it guards, which it
        // yields.
        Op::Guard { trap, fact } => {
            if f.facts.get(fact.0 as usize).is_none() {
                return Err("a guard of a fact that does not exist".into());
            }
            let values = args.len().saturating_sub(2);
            let mut wants = vec![Want::Is(Ty::Bool)];
            wants.extend(std::iter::repeat_n(Want::Int, values));
            wants.push(Want::Int);
            let guarded = args.last().copied().unwrap_or(Ty::Unit);
            (Want::Is(guarded), wants, Some((*trap, values)))
        }
        Op::Assume(fact) => {
            if f.facts.get(fact.0 as usize).is_none() {
                return Err("an assumption of a fact that does not exist".into());
            }
            (same, vec![Want::Int], None)
        }
        Op::Slot { .. } => (Want::Addr, vec![], None),
        Op::Elem { .. } => (Want::Addr, vec![Want::Addr, Want::Int], None),
        Op::Load => (Want::IntOrBool, vec![Want::Addr], None),
        Op::Store => (Want::Is(Ty::Unit), vec![Want::Addr, Want::IntOrBool], None),
        Op::Copy { .. } => (Want::Is(Ty::Unit), vec![Want::Addr, Want::Addr], None),
        Op::Call(c) => {
            let callee = m
                .functions
                .get(c.0 as usize)
                .ok_or("a call of a function that does not exist")?;
            // The callee's arguments; its last parameter, the effect, takes the latest effect.
            let given = &callee.params[..callee.params.len().saturating_sub(1)];
            let params = given.iter().map(|p| Want::param(*p)).collect();
            (Want::Is(callee.result.unwrap_or(Ty::Unit)), params, None)
        }
        Op::Enter { callee, trap } => {
            let counted = m
                .functions
                .get(callee.0 as usize)
                .ok_or("a count of a function that does not exist")?;
            // The arguments as evaluated, before any is narrowed, so of any type: as many as the
            // call passes besides the latest effect.
            let given = &counted.params[..counted.params.len().saturating_sub(1)];
            let params = given.iter().map(|_| Want::Value).collect();
            (Want::Is(Ty::Unit), params, trap.map(|t| (t, 0)))
        }
    };
    let site = trap.is_none_or(|(t, values)| {
        f.traps
            .get(t.0 as usize)
            .is_some_and(|s| s.text.len() == values + 1)
    });
    let typed = result.admits(ty)
        && args.len() == operands.len()
        && operands.iter().zip(&args).all(|(w, t)| w.admits(*t));
    match (typed, site) {
        (true, true) => Ok(()),
        (false, _) => Err(format!("{:?} of type {} on {:?}", f.op(v), ty, args)),
        (true, false) => Err(format!(
            "{:?} traps at a site without room for its values",
            f.op(v)
        )),
    }
}

/// Checks an access against the shape of the memory it reaches ([`Mem`]): a slot takes the
/// bytes of its shape; an element of an array is one of its elements, at their stride, and an
/// element of a record is a field, at its offset; a load gives, and a store takes, the type of
/// the integer or boolean there; and a copy moves all the bytes of memory of one shape between
/// two addresses of it. Where branches meet and where a call passes an address, the address's
/// type ([`Ty::Addr`]) holds the shape, so they agree on it too. So an array built in one shape
/// is never read in another, as one of `i64` read at the stride of `u8` would be.
fn check_memory(mems: &Mems, f: &Function, v: NodeId) -> Result<(), String> {
    let addr = |x: NodeId| {
        f.ty(x)
            .mem()
            .ok_or_else(|| format!("v{} is no address", x.0))
    };
    let arg = |k: usize| {
        f.own_args(v)
            .get(k)
            .copied()
            .ok_or_else(|| format!("no operand {}", k))
    };
    let name = |id: MemId| mems.name(id);
    match f.op(v) {
        Op::Slot { bytes } => {
            let at = addr(v)?;
            if mems.size(at) != Some(*bytes) {
                return Err(format!(
                    "a slot of {} bytes for memory of {}",
                    bytes,
                    name(at)
                ));
            }
        }
        Op::Elem { stride } => {
            let (base, at) = (addr(arg(0)?)?, addr(v)?);
            let fits = match mems.get(base) {
                Some(Mem::Array { elem, .. }) => *elem == at && mems.size(at) == Some(*stride),
                // A field, at the offset a constant index gives.
                Some(Mem::Record { fields, .. }) => match f.fact(arg(1)?) {
                    Some(Fact::Within(k, last)) if *stride == 1 && k == last && !k.neg => {
                        u32::try_from(k.mag).is_ok_and(|k| fields.contains(&(k, at)))
                    }
                    Some(Fact::Within(..) | Fact::NonZero) | None => false,
                },
                Some(Mem::Int(_) | Mem::Bool | Mem::Empty) | None => false,
            };
            if !fits {
                return Err(format!(
                    "an element of memory of {} taken as {} at a stride of {}",
                    name(base),
                    name(at),
                    stride
                ));
            }
        }
        Op::Load => {
            let at = addr(arg(0)?)?;
            if mems.scalar(at) != Some(f.ty(v)) {
                return Err(format!("a load of {} from memory of {}", f.ty(v), name(at)));
            }
        }
        Op::Store => {
            let (at, x) = (addr(arg(0)?)?, f.ty(arg(1)?));
            if mems.scalar(at) != Some(x) {
                return Err(format!("a store of {} into memory of {}", x, name(at)));
            }
        }
        Op::Copy { bytes } => {
            let (to, from) = (addr(arg(0)?)?, addr(arg(1)?)?);
            if to != from || mems.size(to) != Some(*bytes) {
                return Err(format!(
                    "a copy of {} bytes from memory of {} into memory of {}",
                    bytes,
                    name(from),
                    name(to)
                ));
            }
        }
        Op::Param
        | Op::Int(_)
        | Op::Bool(_)
        | Op::Arith { .. }
        | Op::Neg { .. }
        | Op::Bits(_)
        | Op::Not
        | Op::Shl { .. }
        | Op::Shr
        | Op::Cmp(_)
        | Op::Cast
        | Op::Check { .. }
        | Op::Guard { .. }
        | Op::Assume(_)
        | Op::Call(_)
        | Op::Enter { .. } => {}
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------
// The MIR interpreter
// -------------------------------------------------------------------------------------------

/// A value as the MIR interpreter holds it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Val {
    Int(Num),
    Bool(bool),
    Addr(u64),
    Unit,
}

/// Why a run stopped without a value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stop {
    /// A trap, with the line the reference interpreter prints for it.
    Trap(String),
    OutOfSteps,
    /// The MIR broke one of its own rules: a lowering bug, never a program's fault.
    Broken(String),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Trap(line) => write!(f, "{}", line),
            Stop::OutOfSteps => write!(f, "the run took more steps than its limit"),
            Stop::Broken(why) => write!(f, "broken MIR: {}", why),
        }
    }
}

/// Memory, byte-addressed: the slots of every frame, and what a caller sets up. A copy lets
/// one setup start many runs, as the mutation gate's do.
///
/// Each allocation is a slot of its own, and every load, store and copy must stay inside one:
/// an access that crosses into the next slot or past the last is broken MIR, as an index no
/// check kept inside its array makes, never a read of the neighbouring slot's bytes.
#[derive(Clone, Default, Debug)]
pub struct Memory {
    bytes: Vec<u8>,
    /// Where each slot starts and ends, in the order allocated, so by address.
    slots: Vec<(usize, usize)>,
    /// What every new byte holds before it is written: 0 by default. Native code's stack slots
    /// hold whatever was there, so a run that fills them with another byte shows a read of
    /// memory nothing wrote, as the mutation gate's runs do.
    fill: u8,
}

impl Memory {
    /// Memory whose new bytes each hold `fill` until written.
    pub fn filled(fill: u8) -> Memory {
        Memory {
            bytes: Vec::new(),
            slots: Vec::new(),
            fill,
        }
    }

    /// `n` new bytes, aligned to 16, their address returned.
    pub fn alloc(&mut self, n: u32) -> u64 {
        let at = self.bytes.len().next_multiple_of(16);
        self.bytes.resize(at + n as usize, self.fill);
        self.slots.push((at, at + n as usize));
        at as u64
    }

    /// The bytes an access of `n` at `at` covers, which must lie inside one slot.
    fn range(&self, at: u64, n: usize) -> Result<std::ops::Range<usize>, Stop> {
        let start = usize::try_from(at).unwrap_or(usize::MAX);
        // The slot holding `start` is the last that begins at or before it.
        let k = self.slots.partition_point(|(s, _)| *s <= start);
        let slot = k.checked_sub(1).and_then(|k| self.slots.get(k));
        match (start.checked_add(n), slot) {
            (Some(end), Some((_, last))) if end <= *last => Ok(start..end),
            _ => Err(Stop::Broken(format!(
                "an access of {} bytes at {} outside its slot",
                n, at
            ))),
        }
    }

    pub fn store(&mut self, at: u64, v: Val, ty: Ty) -> Result<(), Stop> {
        match (v, ty) {
            (Val::Int(n), Ty::Int(t)) => {
                let r = self.range(at, t.bytes() as usize)?;
                let bits = n.to_bits(t).to_le_bytes();
                self.bytes[r].copy_from_slice(&bits[..t.bytes() as usize]);
            }
            (Val::Bool(b), Ty::Bool) => {
                let r = self.range(at, 1)?;
                self.bytes[r.start] = u8::from(b);
            }
            _ => return Err(Stop::Broken(format!("a store of {:?} as {}", v, ty))),
        }
        Ok(())
    }

    pub fn load(&self, at: u64, ty: Ty) -> Result<Val, Stop> {
        match ty {
            Ty::Int(t) => {
                let r = self.range(at, t.bytes() as usize)?;
                let mut bits = [0u8; 16];
                bits[..t.bytes() as usize].copy_from_slice(&self.bytes[r]);
                Ok(Val::Int(Num::from_bits(u128::from_le_bytes(bits), t)))
            }
            Ty::Bool => Ok(Val::Bool(self.bytes[self.range(at, 1)?.start] != 0)),
            Ty::Addr(_) | Ty::Unit | Ty::Effect => Err(Stop::Broken(format!("a load of {}", ty))),
        }
    }

    fn copy(&mut self, dst: u64, src: u64, n: u32) -> Result<(), Stop> {
        let s = self.range(src, n as usize)?;
        let d = self.range(dst, n as usize)?;
        self.bytes.copy_within(s, d.start);
        Ok(())
    }

    /// Where memory stands, for a call to give back on its return.
    fn mark(&self) -> Mark {
        Mark {
            bytes: self.bytes.len(),
            slots: self.slots.len(),
        }
    }

    /// Gives back the bytes and the slots allocated since `to`. Slots are counted, not found by
    /// address: a slot of no bytes made just before a call begins at the address the call's own
    /// memory does, yet belongs to the caller.
    fn release(&mut self, to: Mark) {
        self.bytes.truncate(to.bytes);
        self.slots.truncate(to.slots);
    }

    /// Bytes in use: what a caller set up, and the slots of the calls still running.
    pub fn used(&self) -> usize {
        self.bytes.len()
    }
}

/// How much memory, in bytes and in slots, was in use when a call began.
#[derive(Clone, Copy, Debug)]
struct Mark {
    bytes: usize,
    slots: usize,
}

/// The resources a run may use. Its stack is no limit of these: the module's budget counts it
/// (`Module::budget`).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_steps: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: 100_000_000,
        }
    }
}

/// Runs function `f` of `m` on `args`, with memory a caller has set up for its addresses. Calls
/// keep their frames on a stack of the interpreter's own, so a deep recursion never reaches the
/// host's stack: its [`Op::Enter`]s trap against the module's budget first. `f` itself is the
/// run's entry, which no frame of the program counts; every call of a function of the program,
/// the run's first among them, is counted by its `Enter`. `args` are the entry's arguments; the
/// run begins the chain of effects, its entry's effect parameter taking no value of its own.
pub fn run(
    m: &Module,
    f: FuncId,
    args: &[Val],
    memory: &mut Memory,
    limits: Limits,
) -> Result<Val, Stop> {
    run_counting(m, f, args, memory, limits).0
}

/// Runs as [`run`] does, and also gives the number of instructions the run executed, so that
/// a caller can bound another run by what this one took.
pub fn run_counting(
    m: &Module,
    f: FuncId,
    args: &[Val],
    memory: &mut Memory,
    limits: Limits,
) -> (Result<Val, Stop>, u64) {
    let mut machine = Machine {
        m,
        memory,
        limits,
        steps: 0,
        stack: 0,
    };
    let result = machine.run(f, args);
    (result, machine.steps)
}

struct Machine<'a> {
    m: &'a Module,
    memory: &'a mut Memory,
    limits: Limits,
    steps: u64,
    /// The bytes the frames of the calls counted and not returned sum to.
    stack: u128,
}

/// A call in progress: its function, its values, where it stands, the memory it started at,
/// and the value of its caller its result goes to.
struct Frame<'m> {
    f: &'m Function,
    vals: Vec<Val>,
    block: BlockId,
    next: usize,
    mark: Mark,
    dest: Option<NodeId>,
}

fn broken(why: impl Into<String>) -> Stop {
    Stop::Broken(why.into())
}

/// Enters block `b` of a frame with `args` as its parameters.
fn enter(frame: &mut Frame, b: BlockId, args: Vec<Val>) -> Result<(), Stop> {
    let params = &frame
        .f
        .blocks
        .get(b.0 as usize)
        .ok_or_else(|| broken("a jump to a block that does not exist"))?
        .params;
    if params.len() != args.len() {
        return Err(broken("a block entered with the wrong arguments"));
    }
    for (p, v) in params.iter().zip(args) {
        frame.vals[p.0 as usize] = v;
    }
    frame.block = b;
    frame.next = 0;
    Ok(())
}

impl<'m> Machine<'m> {
    fn frame(&self, id: FuncId, args: Vec<Val>, dest: Option<NodeId>) -> Result<Frame<'m>, Stop> {
        let f = self
            .m
            .functions
            .get(id.0 as usize)
            .ok_or_else(|| broken("a call of a function that does not exist"))?;
        let mut frame = Frame {
            f,
            vals: vec![Val::Unit; f.ops.len()],
            block: BlockId(0),
            next: 0,
            mark: self.memory.mark(),
            dest,
        };
        // The latest effect before the call, which the interpreter keeps no value for.
        let mut args = args;
        args.push(Val::Unit);
        enter(&mut frame, BlockId(0), args)?;
        Ok(frame)
    }

    fn run(&mut self, id: FuncId, args: &[Val]) -> Result<Val, Stop> {
        let mut stack = vec![self.frame(id, args.to_vec(), None)?];
        loop {
            let Some(top) = stack.last_mut() else {
                return Err(broken("a return with no frame"));
            };
            let f = top.f;
            let b = f.block(top.block);
            if let Some(&v) = b.insts.get(top.next) {
                top.next += 1;
                self.steps += 1;
                if self.steps > self.limits.max_steps {
                    return Err(Stop::OutOfSteps);
                }
                if let Op::Call(c) = f.op(v) {
                    let args = f
                        .own_args(v)
                        .iter()
                        .map(|a| top.vals[a.0 as usize])
                        .collect();
                    let callee = self.frame(*c, args, Some(v))?;
                    stack.push(callee);
                    continue;
                }
                let value = self.inst(f, v, &top.vals)?;
                top.vals[v.0 as usize] = value;
                continue;
            }
            let get = |v: &NodeId| top.vals[v.0 as usize];
            match &b.term {
                Term::Jump(t, args) => {
                    let args = args.iter().map(get).collect();
                    enter(top, *t, args)?;
                }
                Term::Branch {
                    cond,
                    then,
                    otherwise,
                } => {
                    let (t, args) = match get(cond) {
                        Val::Bool(true) => then,
                        Val::Bool(false) => otherwise,
                        Val::Int(_) | Val::Addr(_) | Val::Unit => {
                            return Err(broken("a branch on a value that is not a bool"))
                        }
                    };
                    let args = args.iter().map(get).collect();
                    enter(top, *t, args)?;
                }
                Term::Return(v, _) => {
                    let result = v.as_ref().map_or(Val::Unit, get);
                    let (mark, dest) = (top.mark, top.dest);
                    // A call gives back the bytes its `Enter` counted; the entry counted none.
                    if dest.is_some() {
                        self.stack -= f.frame;
                    }
                    stack.pop();
                    self.memory.release(mark);
                    match (stack.last_mut(), dest) {
                        (Some(caller), Some(d)) => caller.vals[d.0 as usize] = result,
                        _ => return Ok(result),
                    }
                }
                Term::Trap(t, _) => return Err(trap(f, *t, &[])),
            }
        }
    }

    fn inst(&mut self, f: &Function, v: NodeId, vals: &[Val]) -> Result<Val, Stop> {
        let args: Vec<Val> = f.own_args(v).iter().map(|a| vals[a.0 as usize]).collect();
        let ty = f.ty(v);
        let int = |i: usize| -> Result<Num, Stop> {
            match args.get(i) {
                Some(Val::Int(n)) => Ok(*n),
                _ => Err(broken(format!("v{} wants an integer operand", v.0))),
            }
        };
        let addr = |i: usize| -> Result<u64, Stop> {
            match args.get(i) {
                Some(Val::Addr(a)) => Ok(*a),
                _ => Err(broken(format!("v{} wants an address operand", v.0))),
            }
        };
        // An integer result of the value's type: an exact one must fit, a checked one traps.
        let fit = |n: Option<Num>, trap_at: Option<TrapId>| -> Result<Val, Stop> {
            let Ty::Int(t) = ty else {
                return Err(broken(format!("v{} is not an integer", v.0)));
            };
            match (n.filter(|n| t.holds(*n)), trap_at) {
                (Some(n), _) => Ok(Val::Int(n)),
                (None, Some(at)) => Err(trap(f, at, &[n])),
                (None, None) => Err(broken(format!("the exact v{} left its type", v.0))),
            }
        };
        Ok(match f.op(v) {
            Op::Param => return Err(broken("a parameter in a block's instructions")),
            Op::Int(bits) => match ty {
                Ty::Int(t) => Val::Int(Num::from_bits(*bits, t)),
                Ty::Bool | Ty::Addr(_) | Ty::Unit | Ty::Effect => {
                    return Err(broken("an integer of a type that is not one"))
                }
            },
            Op::Bool(b) => Val::Bool(*b),
            Op::Arith { op, trap } => {
                let (a, b) = (int(0)?, int(1)?);
                let n = match op {
                    ArithOp::Add => a.add(b),
                    ArithOp::Sub => a.add(b.negated()),
                    ArithOp::Mul => a.mul(b),
                    ArithOp::Div | ArithOp::Rem if b == Num::ZERO => {
                        return Err(broken(format!("v{} divides by zero unchecked", v.0)))
                    }
                    ArithOp::Div => Some(a.div_rem(b).0),
                    ArithOp::Rem => Some(a.div_rem(b).1),
                };
                fit(n, *trap)?
            }
            Op::Neg { trap } => fit(Some(int(0)?.negated()), *trap)?,
            Op::Bits(op) => match (args.first(), args.get(1), ty) {
                (Some(Val::Bool(a)), Some(Val::Bool(b)), _) => Val::Bool(match op {
                    BitOp::And => *a && *b,
                    BitOp::Or => *a || *b,
                    BitOp::Xor => a != b,
                }),
                (_, _, Ty::Int(t)) => {
                    let (a, b) = (int(0)?.to_bits(t), int(1)?.to_bits(t));
                    let bits = match op {
                        BitOp::And => a & b,
                        BitOp::Or => a | b,
                        BitOp::Xor => a ^ b,
                    };
                    Val::Int(Num::from_bits(bits, t))
                }
                _ => return Err(broken("bits on a value that is no integer or bool")),
            },
            Op::Not => match (args.first(), ty) {
                (Some(Val::Bool(a)), _) => Val::Bool(!*a),
                (_, Ty::Int(t)) => Val::Int(Num::from_bits(!int(0)?.to_bits(t), t)),
                _ => return Err(broken("not on a value that is no integer or bool")),
            },
            Op::Shl { trap: at } => {
                let (a, k) = (int(0)?, shift(int(1)?)?);
                let n = a.mag.checked_mul(1u128 << k).map(|m| Num::new(a.neg, m));
                match (n, ty) {
                    (Some(n), Ty::Int(t)) if t.holds(n) => Val::Int(n),
                    (n, Ty::Int(_)) => return Err(trap(f, *at, &[n])),
                    _ => return Err(broken("a shift of a value that is no integer")),
                }
            }
            Op::Shr => {
                let (a, k) = (int(0)?, shift(int(1)?)?);
                // Floor: a negative value whose low bits are not all zero rounds down.
                let mag = a.mag >> k;
                let exact = mag << k == a.mag;
                Val::Int(if a.neg && !exact {
                    Num::new(true, mag + 1)
                } else {
                    Num::new(a.neg, mag)
                })
            }
            Op::Cmp(op) => {
                let ord = match (args.first(), args.get(1)) {
                    (Some(Val::Int(a)), Some(Val::Int(b))) => a.cmp(b),
                    (Some(Val::Bool(a)), Some(Val::Bool(b))) => a.cmp(b),
                    _ => return Err(broken("a comparison of values that are not alike")),
                };
                Val::Bool(match op {
                    CmpOp::Eq => ord.is_eq(),
                    CmpOp::Ne => ord.is_ne(),
                    CmpOp::Lt => ord.is_lt(),
                    CmpOp::Le => ord.is_le(),
                    CmpOp::Gt => ord.is_gt(),
                    CmpOp::Ge => ord.is_ge(),
                })
            }
            Op::Cast => fit(Some(int(0)?), None)?,
            Op::Check { trap: at } => match args.first() {
                Some(Val::Bool(true)) => Val::Unit,
                Some(Val::Bool(false)) => {
                    let shown: Vec<Option<Num>> = args[1..]
                        .iter()
                        .map(|a| match a {
                            Val::Int(n) => Some(*n),
                            Val::Bool(_) | Val::Addr(_) | Val::Unit => None,
                        })
                        .collect();
                    return Err(trap(f, *at, &shown));
                }
                _ => return Err(broken("a check of a value that is not a bool")),
            },
            // A guard traps as a check does, then yields the value it guards, of which its fact
            // must hold: a guard whose condition lets through what its fact does not is broken.
            Op::Guard { trap: at, fact } => {
                let fact = f
                    .facts
                    .get(fact.0 as usize)
                    .copied()
                    .ok_or_else(|| broken("a guard of a fact that does not exist"))?;
                let cond = match args.first() {
                    Some(Val::Bool(c)) => Some(*c),
                    Some(Val::Int(_) | Val::Addr(_) | Val::Unit) | None => None,
                };
                match cond {
                    Some(true) => {}
                    Some(false) => {
                        let reported = &args[1..args.len().saturating_sub(1)];
                        let shown: Vec<Option<Num>> = reported
                            .iter()
                            .map(|a| match a {
                                Val::Int(n) => Some(*n),
                                Val::Bool(_) | Val::Addr(_) | Val::Unit => None,
                            })
                            .collect();
                        return Err(trap(f, *at, &shown));
                    }
                    None => return Err(broken("a guard of a value that is not a bool")),
                }
                let n = int(args.len().saturating_sub(1))?;
                if !fact.holds(n) {
                    return Err(broken(format!(
                        "v{} lets through {}, of which {} does not hold",
                        v.0, n, fact
                    )));
                }
                Val::Int(n)
            }
            Op::Assume(fact) => match (
                int(0)?,
                f.facts
                    .get(fact.0 as usize)
                    .copied()
                    .ok_or_else(|| broken("an assumption of a fact that does not exist"))?,
            ) {
                (n, fact) if fact.holds(n) => Val::Int(n),
                (n, fact) => {
                    return Err(broken(format!(
                        "v{} assumes {} of {}, which it does not hold of",
                        v.0, fact, n
                    )))
                }
            },
            Op::Slot { bytes } => Val::Addr(self.memory.alloc(*bytes)),
            Op::Elem { stride } => {
                let i = int(1)?;
                if i.neg {
                    return Err(broken("a negative index unchecked"));
                }
                let base = addr(0)?;
                let at = u64::try_from(i.mag)
                    .ok()
                    .and_then(|i| i.checked_mul(u64::from(*stride)))
                    .and_then(|offset| base.checked_add(offset))
                    .ok_or_else(|| broken("an index beyond memory"))?;
                Val::Addr(at)
            }
            Op::Load => self.memory.load(addr(0)?, ty)?,
            Op::Store => {
                let at = addr(0)?;
                let ty = f.ty(f.args(v)[1]);
                self.memory.store(at, args[1], ty)?;
                Val::Unit
            }
            Op::Copy { bytes } => {
                self.memory.copy(addr(0)?, addr(1)?, *bytes)?;
                Val::Unit
            }
            Op::Call(_) => return Err(broken("a call outside the frame loop")),
            Op::Enter { callee, trap: at } => {
                if self.m.runs_out(self.stack, *callee) {
                    return Err(match at {
                        Some(at) => trap(f, *at, &[]),
                        None => broken("an exact count ran out of the budget"),
                    });
                }
                self.stack += self.m.frame_of(*callee);
                Val::Unit
            }
        })
    }
}

/// A shift amount, which a check has kept within the type's width.
fn shift(k: Num) -> Result<u32, Stop> {
    if k.neg || k.mag >= 128 {
        return Err(broken("a shift amount unchecked"));
    }
    Ok(k.mag as u32)
}

fn trap(f: &Function, at: TrapId, values: &[Option<Num>]) -> Stop {
    let Some(site) = f.traps.get(at.0 as usize) else {
        return broken("a trap at a site that does not exist");
    };
    let mut message = site.text.first().cloned().unwrap_or_default();
    for (v, piece) in values.iter().zip(&site.text[1..]) {
        match v {
            Some(n) => message.push_str(&n.to_string()),
            None => message.push_str("the result"),
        }
        message.push_str(piece);
    }
    Stop::Trap(format!(
        "trap[{}] at {}:{}: {}",
        site.kind, site.loc.line, site.loc.col, message
    ))
}

// -------------------------------------------------------------------------------------------
// Printing
// -------------------------------------------------------------------------------------------

impl fmt::Display for Function {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |vs: &[NodeId]| {
            vs.iter()
                .map(|v| format!("v{}", v.0))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let params: Vec<String> = self.params.iter().map(Ty::to_string).collect();
        write!(out, "fn {}({})", self.name, params.join(", "))?;
        if let Some(r) = self.result {
            write!(out, " -> {}", r)?;
        }
        if self.frame > 0 {
            write!(out, " frame {}", self.frame)?;
        }
        writeln!(out, " {{")?;
        for (i, b) in self.blocks.iter().enumerate() {
            let params: Vec<String> = b
                .params
                .iter()
                .map(|p| format!("v{}: {}", p.0, self.ty(*p)))
                .collect();
            writeln!(out, "block{}({}):", i, params.join(", "))?;
            for &v in &b.insts {
                let loc = self.locs[v.0 as usize];
                let mut op = self.op_text(v);
                if !self.args(v).is_empty() {
                    op = format!("{} {}", op, list(self.args(v)));
                }
                writeln!(
                    out,
                    "  v{}: {} = {} @{}:{}",
                    v.0,
                    self.ty(v),
                    op,
                    loc.line,
                    loc.col
                )?;
            }
            let jump = |(t, args): &(BlockId, Vec<NodeId>)| format!("block{}({})", t.0, list(args));
            match &b.term {
                Term::Jump(t, args) => writeln!(out, "  jump {}", jump(&(*t, args.clone())))?,
                Term::Branch {
                    cond,
                    then,
                    otherwise,
                } => writeln!(
                    out,
                    "  branch v{}, {}, {}",
                    cond.0,
                    jump(then),
                    jump(otherwise)
                )?,
                Term::Return(Some(v), e) => writeln!(out, "  return v{} after v{}", v.0, e.0)?,
                Term::Return(None, e) => writeln!(out, "  return after v{}", e.0)?,
                Term::Trap(t, e) => writeln!(out, "  trap t{} after v{}", t.0, e.0)?,
            }
        }
        for (i, t) in self.traps.iter().enumerate() {
            writeln!(
                out,
                "t{}: {} @{}:{} {:?}",
                i, t.kind, t.loc.line, t.loc.col, t.text
            )?;
        }
        writeln!(out, "}}")
    }
}

/// Each function in order, numbered as a call names it, with a blank line between two: the
/// canonical text of a module, which `mn dump --mir` prints.
impl fmt::Display for Module {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, f) in self.functions.iter().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            write!(out, "f{}: {}", i, f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U8: IntTy = IntTy::new(8, false);
    const I64: IntTy = IntTy::new(64, true);
    const U64: IntTy = IntTy::new(64, false);
    const U128: IntTy = IntTy::new(128, false);
    const I128: IntTy = IntTy::new(128, true);

    fn int(n: i128) -> Val {
        Val::Int(Num::from_i128(n))
    }

    /// The budget of the modules these tests build.
    const BUDGET: u128 = 1 << 20;

    /// The shapes of memory these tests' addresses point at, each with its address's type below.
    fn mems() -> Mems {
        let mut m = Mems::default();
        let byte = m.of(Mem::Int(U8));
        m.of(Mem::Array { len: 3, elem: byte });
        m.of(Mem::Bool);
        m.of(Mem::Array { len: 4, elem: byte });
        m.of(Mem::Int(IntTy::new(16, false)));
        m.of(Mem::Int(I64));
        m
    }
    const BYTE: Ty = Ty::Addr(MemId(0));
    const BYTES3: Ty = Ty::Addr(MemId(1));
    const FLAG: Ty = Ty::Addr(MemId(2));
    const BYTES4: Ty = Ty::Addr(MemId(3));
    const WORD: Ty = Ty::Addr(MemId(4));
    const LONG: Ty = Ty::Addr(MemId(5));

    fn module(functions: Vec<Function>) -> Module {
        let m = Module {
            functions,
            budget: BUDGET,
            mems: mems(),
        };
        verify(&m).unwrap();
        m
    }

    /// A call of function `callee` on `args` where `b` stands, counted first as the lowering counts
    /// it: the count takes the arguments, and the call takes the count.
    fn call(b: &mut Builder, callee: u32, ty: Ty, args: &[NodeId]) -> NodeId {
        let trap = b.trap_site("stack-overflow", &["the call stack ran out"]);
        let count = b.inst(
            Op::Enter {
                callee: FuncId(callee),
                trap: Some(trap),
            },
            Ty::Unit,
            args,
        );
        let _ = count;
        b.inst(Op::Call(FuncId(callee)), ty, args)
    }

    fn run0(m: &Module, f: u32, args: &[Val]) -> Result<Val, Stop> {
        run(
            m,
            FuncId(f),
            args,
            &mut Memory::default(),
            Limits::default(),
        )
    }

    fn add(trap: Option<TrapId>) -> Op {
        Op::Arith {
            op: ArithOp::Add,
            trap,
        }
    }

    /// `fn sum(n: i64) -> i64`: 0 + 1 + ... + (n - 1), through a loop of blocks.
    fn sum() -> Function {
        let mut b = Builder::new("sum", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let n = b.params(BlockId(0))[0];
        let zero = b.int(I64, Num::ZERO);
        let head = b.block(&[Ty::Int(I64), Ty::Int(I64)]);
        let body = b.block(&[]);
        let exit = b.block(&[]);
        b.jump(head, vec![zero, zero]);
        b.switch(head);
        let [i, s] = b.params(head)[..] else {
            panic!("two parameters")
        };
        let more = b.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n]);
        b.branch(more, (body, vec![]), (exit, vec![]));
        b.switch(body);
        let s2 = b.inst(add(None), Ty::Int(I64), &[s, i]);
        let one = b.int(I64, Num::from_i128(1));
        let i2 = b.inst(add(None), Ty::Int(I64), &[i, one]);
        b.jump(head, vec![i2, s2]);
        b.switch(exit);
        b.ret(Some(s));
        b.finish()
    }

    #[test]
    fn test_a_function_runs_through_its_blocks() {
        let m = module(vec![sum()]);
        assert_eq!(run0(&m, 0, &[int(10)]), Ok(int(45)));
        assert_eq!(run0(&m, 0, &[int(0)]), Ok(int(0)));
        let printed = m.functions[0].to_string();
        assert!(
            printed.starts_with(
                "fn sum(i64, effect) -> i64 {\nblock0(v0: i64, v1: effect):\n  v2: i64 = int 0 @0:0\n"
            ),
            "{}",
            printed
        );
        assert!(
            printed.contains("  branch v8, block2(v5), block3(v5)\n"),
            "{}",
            printed
        );
        assert!(
            printed.contains("  v9: i64 = add v4, v3, v6 @0:0\n"),
            "{}",
            printed
        );
    }

    #[test]
    fn test_a_module_prints_each_function_numbered_as_calls_name_it() {
        let mut b = Builder::new("twice", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let n = b.params(BlockId(0))[0];
        let s = call(&mut b, 0, Ty::Int(I64), &[n]);
        b.ret(Some(s));
        let m = module(vec![sum(), b.finish()]);
        let printed = m.to_string();
        let first = m.functions[0].to_string();
        let second = m.functions[1].to_string();
        assert_eq!(printed, format!("f0: {}\nf1: {}", first, second));
        assert!(second.contains("= enter f0 t0 v0, v1 @0:0\n"), "{}", second);
        assert!(second.contains("= call f0 v0, v2 @0:0\n"), "{}", second);
        assert!(second.contains("  return v3 after v3\n"), "{}", second);
        // The same module prints the same text every time.
        assert_eq!(printed, m.to_string());
        assert_eq!(Module::default().to_string(), "");
    }

    #[test]
    fn test_checks_trap_with_the_interpreters_message() {
        // `val y: u8 = x`: a check that x fits, then the cast.
        let mut b = Builder::new("narrow", &[Ty::Int(I64)], Some(Ty::Int(U8)));
        let x = b.params(BlockId(0))[0];
        b.at(3, 17);
        let lo = b.int(I64, Num::ZERO);
        let hi = b.int(I64, Num::from_i128(255));
        let ge = b.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[x, lo]);
        let le = b.inst(Op::Cmp(CmpOp::Le), Ty::Bool, &[x, hi]);
        let fits = b.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, le]);
        let t = b.trap_site("narrowing", &["", " does not fit in `u8`"]);
        let fact = Fact::Within(Num::ZERO, Num::from_i128(255));
        let g = b.guard(t, fact, &[fits, x, x]);
        let y = b.inst(Op::Cast, Ty::Int(U8), &[g]);
        b.ret(Some(y));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(200)]), Ok(int(200)));
        assert_eq!(
            run0(&m, 0, &[int(-1)]).unwrap_err().to_string(),
            "trap[narrowing] at 3:17: -1 does not fit in `u8`"
        );
        // A guard whose condition lets through a value its fact does not hold of is broken MIR.
        let mut b = Builder::new("loose", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = b.params(BlockId(0))[0];
        let yes = b.inst(Op::Bool(true), Ty::Bool, &[]);
        let t = b.trap_site("narrowing", &["", " does not fit in `u8`"]);
        let fact = Fact::Within(Num::ZERO, Num::from_i128(255));
        let g = b.guard(t, fact, &[yes, x, x]);
        b.ret(Some(g));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(255)]), Ok(int(255)));
        assert!(matches!(
            run0(&m, 0, &[int(256)]),
            Err(Stop::Broken(why)) if why.contains("256, of which within 0..=255 does not hold")
        ));
        // An assumption that does not hold of what it takes is broken MIR, though nothing after it
        // would be.
        let mut b = Builder::new("assumed", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = b.params(BlockId(0))[0];
        let a = b.assume(x, Fact::Within(Num::ZERO, Num::from_i128(255)));
        b.ret(Some(a));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(255)]), Ok(int(255)));
        assert!(matches!(
            run0(&m, 0, &[int(256)]),
            Err(Stop::Broken(why)) if why.contains("assumes within 0..=255 of 256")
        ));
        // A division the verifier would refuse, run as it is: a zero divisor is broken MIR, and
        // never a panic of the interpreter.
        let mut b = Builder::new("bare", &[Ty::Int(I64), Ty::Int(I64)], Some(Ty::Int(I64)));
        let [x, d] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let q = b.inst(
            Op::Arith {
                op: ArithOp::Div,
                trap: None,
            },
            Ty::Int(I64),
            &[x, d],
        );
        b.ret(Some(q));
        let unverified = Module {
            functions: vec![b.finish()],
            budget: BUDGET,
            mems: mems(),
        };
        assert!(verify(&unverified).is_err());
        assert!(matches!(
            run0(&unverified, 0, &[int(1), int(0)]),
            Err(Stop::Broken(why)) if why.contains("divides by zero unchecked")
        ));
        // A refinement whose bound is a parameter reports the value and both bounds.
        let mut r = Builder::new("refined", &[Ty::Int(I64), Ty::Int(I64)], None);
        let [x, n] = r.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        r.at(4, 9);
        let zero = r.int(I64, Num::ZERO);
        let below = r.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[x, n]);
        let site = r.trap_site(
            "refinement",
            &["", " lies outside the refinement `[", "..", "]`"],
        );
        r.inst(Op::Check { trap: site }, Ty::Unit, &[below, x, zero, n]);
        r.ret(None);
        let m = module(vec![r.finish()]);
        assert_eq!(run0(&m, 0, &[int(2), int(3)]), Ok(Val::Unit));
        assert_eq!(
            run0(&m, 0, &[int(5), int(3)]).unwrap_err().to_string(),
            "trap[refinement] at 4:9: 5 lies outside the refinement `[0..3]`"
        );
        // A check that reports no value, and a trap terminator, as `todo` lowers.
        let mut t = Builder::new("todo", &[Ty::Bool], Some(Ty::Int(U8)));
        let c = t.params(BlockId(0))[0];
        t.at(2, 5);
        let zero = t.trap_site("division-by-zero", &["division by zero"]);
        t.inst(Op::Check { trap: zero }, Ty::Unit, &[c]);
        let site = t.trap_site("todo", &["todo reached: later"]);
        t.trap(site);
        let m = module(vec![t.finish()]);
        assert_eq!(
            run0(&m, 0, &[Val::Bool(true)]),
            Err(Stop::Trap("trap[todo] at 2:5: todo reached: later".into()))
        );
        assert_eq!(
            run0(&m, 0, &[Val::Bool(false)]),
            Err(Stop::Trap(
                "trap[division-by-zero] at 2:5: division by zero".into()
            ))
        );
    }

    #[test]
    fn test_a_checked_operation_takes_operands_of_their_own_types() {
        // An `i64` times a `u128`, checked against `u128`: computed exactly, then checked.
        let product = |checked: bool| {
            let mut b = Builder::new("mul", &[Ty::Int(I64), Ty::Int(U128)], Some(Ty::Int(U128)));
            let [x, y] = b.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            let trap = checked.then(|| b.trap_site("narrowing", &["", " does not fit in `u128`"]));
            let p = b.inst(
                Op::Arith {
                    op: ArithOp::Mul,
                    trap,
                },
                Ty::Int(U128),
                &[x, y],
            );
            b.ret(Some(p));
            Module {
                functions: vec![b.finish()],
                budget: BUDGET,
                mems: mems(),
            }
        };
        let m = product(true);
        verify(&m).unwrap();
        assert_eq!(run0(&m, 0, &[int(-3), int(0)]), Ok(int(0)));
        assert_eq!(run0(&m, 0, &[int(3), int(4)]), Ok(int(12)));
        assert_eq!(
            run0(&m, 0, &[int(-3), int(4)]).unwrap_err().to_string(),
            "trap[narrowing] at 0:0: -12 does not fit in `u128`"
        );
        // An exact operation's operands share its type.
        assert!(verify(&product(false)).is_err());
    }

    #[test]
    fn test_checked_arithmetic_traps_where_exact_arithmetic_is_a_lowering_bug() {
        let sum2 = |checked: bool, t: IntTy| {
            let mut b = Builder::new("add", &[Ty::Int(t), Ty::Int(t)], Some(Ty::Int(t)));
            let [x, y] = b.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            b.at(2, 30);
            let trap = checked.then(|| b.trap_site("narrowing", &["", " does not fit in `u128`"]));
            let s = b.inst(add(trap), Ty::Int(t), &[x, y]);
            b.ret(Some(s));
            module(vec![b.finish()])
        };
        let top = Val::Int(U128.max());
        assert_eq!(
            run0(&sum2(true, U128), 0, &[top, int(1)])
                .unwrap_err()
                .to_string(),
            "trap[narrowing] at 2:30: the result does not fit in `u128`"
        );
        assert_eq!(run0(&sum2(true, U128), 0, &[int(1), int(2)]), Ok(int(3)));
        // A checked difference below zero reports its exact value, as the interpreter does.
        let mut b = Builder::new("sub", &[Ty::Int(U128), Ty::Int(U128)], Some(Ty::Int(U128)));
        let [x, y] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let t = b.trap_site("narrowing", &["", " does not fit in `u128`"]);
        let d = b.inst(
            Op::Arith {
                op: ArithOp::Sub,
                trap: Some(t),
            },
            Ty::Int(U128),
            &[x, y],
        );
        b.ret(Some(d));
        let m = module(vec![b.finish()]);
        assert_eq!(
            run0(&m, 0, &[int(1), int(2)]).unwrap_err().to_string(),
            "trap[narrowing] at 0:0: -1 does not fit in `u128`"
        );
        assert!(matches!(
            run0(&sum2(false, U64), 0, &[Val::Int(U64.max()), int(1)]),
            Err(Stop::Broken(_))
        ));
        assert_eq!(run0(&sum2(false, I128), 0, &[int(-3), int(5)]), Ok(int(2)));
        // A checked negation of the smallest i128.
        let mut b = Builder::new("neg", &[Ty::Int(I128)], Some(Ty::Int(I128)));
        let x = b.params(BlockId(0))[0];
        let t = b.trap_site("narrowing", &["", " does not fit in `i128`"]);
        let r = b.inst(Op::Neg { trap: Some(t) }, Ty::Int(I128), &[x]);
        b.ret(Some(r));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(5)]), Ok(int(-5)));
        assert!(matches!(run0(&m, 0, &[int(i128::MIN)]), Err(Stop::Trap(_))));
    }

    #[test]
    fn test_arithmetic_and_bits_follow_the_core_calculus() {
        let binary = |op: Op, t: IntTy, a: i128, b: i128| {
            let mut bl = Builder::new("op", &[Ty::Int(t), Ty::Int(t)], Some(Ty::Int(t)));
            let [x, y] = bl.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            let need = if matches!(
                op,
                Op::Arith {
                    op: ArithOp::Div | ArithOp::Rem,
                    ..
                }
            ) {
                Some(Fact::NonZero)
            } else if op == Op::Shr {
                Some(Fact::Within(Num::ZERO, Num::new(false, 127)))
            } else {
                None
            };
            let y = match need {
                Some(fact) => bl.assume(y, fact),
                None => y,
            };
            let r = bl.inst(op, Ty::Int(t), &[x, y]);
            bl.ret(Some(r));
            run0(&module(vec![bl.finish()]), 0, &[int(a), int(b)])
        };
        let arith = |op| Op::Arith { op, trap: None };
        assert_eq!(binary(arith(ArithOp::Sub), I64, 3, 5), Ok(int(-2)));
        assert_eq!(binary(arith(ArithOp::Sub), I64, 5, 3), Ok(int(2)));
        assert_eq!(binary(arith(ArithOp::Add), I64, -5, 3), Ok(int(-2)));
        assert_eq!(binary(arith(ArithOp::Mul), I64, -3, 5), Ok(int(-15)));
        assert_eq!(binary(arith(ArithOp::Mul), I64, -3, -5), Ok(int(15)));
        assert_eq!(binary(arith(ArithOp::Div), I64, -7, 2), Ok(int(-3)));
        assert_eq!(binary(arith(ArithOp::Rem), I64, -7, 2), Ok(int(-1)));
        assert_eq!(binary(arith(ArithOp::Rem), I64, 7, -2), Ok(int(1)));
        assert!(matches!(
            binary(arith(ArithOp::Div), I64, 1, 0),
            Err(Stop::Broken(_))
        ));
        assert_eq!(binary(Op::Bits(BitOp::And), U8, 12, 10), Ok(int(8)));
        assert_eq!(binary(Op::Bits(BitOp::Or), U8, 12, 10), Ok(int(14)));
        assert_eq!(binary(Op::Bits(BitOp::Xor), U8, 12, 10), Ok(int(6)));
        assert_eq!(binary(Op::Bits(BitOp::And), I64, -1, 7), Ok(int(7)));
        assert_eq!(binary(Op::Shr, I64, -7, 1), Ok(int(-4)));
        assert_eq!(binary(Op::Shr, I64, -8, 1), Ok(int(-4)));
        assert_eq!(binary(Op::Shr, I64, 7, 1), Ok(int(3)));
        assert_eq!(binary(Op::Shr, I64, -1, 63), Ok(int(-1)));
        let mut shl = Builder::new("shl", &[Ty::Int(I128), Ty::Int(I64)], Some(Ty::Int(I64)));
        let [x, k] = shl.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let t = shl.trap_site("narrowing", &["", " does not fit in `i64`"]);
        let k = shl.assume(k, Fact::Within(Num::ZERO, Num::new(false, 127)));
        let r = shl.inst(Op::Shl { trap: t }, Ty::Int(I64), &[x, k]);
        shl.ret(Some(r));
        let m = module(vec![shl.finish()]);
        assert_eq!(run0(&m, 0, &[int(-3), int(2)]), Ok(int(-12)));
        // A wider operand whose shifted value fits the result type.
        assert_eq!(
            run0(&m, 0, &[int(1 << 70), int(0)])
                .unwrap_err()
                .to_string(),
            "trap[narrowing] at 0:0: 1180591620717411303424 does not fit in `i64`"
        );
        // The exact value is reported while it lies below 2^128, and "the result" after.
        assert_eq!(
            run0(&m, 0, &[int(1), int(63)]).unwrap_err().to_string(),
            "trap[narrowing] at 0:0: 9223372036854775808 does not fit in `i64`"
        );
        let mut wide = Builder::new("wide", &[Ty::Int(I128), Ty::Int(I128)], Some(Ty::Int(I128)));
        let [x, k] = wide.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let t = wide.trap_site("narrowing", &["", " does not fit in `i128`"]);
        let k = wide.assume(k, Fact::Within(Num::ZERO, Num::new(false, 127)));
        let r = wide.inst(Op::Shl { trap: t }, Ty::Int(I128), &[x, k]);
        wide.ret(Some(r));
        let m = module(vec![wide.finish()]);
        assert_eq!(
            run0(&m, 0, &[int(1 << 126), int(1)]).unwrap_err().to_string(),
            "trap[narrowing] at 0:0: 170141183460469231731687303715884105728 does not fit in `i128`"
        );
        assert_eq!(
            run0(&m, 0, &[int(1 << 126), int(2)])
                .unwrap_err()
                .to_string(),
            "trap[narrowing] at 0:0: the result does not fit in `i128`"
        );
        let mut not = Builder::new("not", &[Ty::Int(U8)], Some(Ty::Int(U8)));
        let x = not.params(BlockId(0))[0];
        let r = not.inst(Op::Not, Ty::Int(U8), &[x]);
        not.ret(Some(r));
        assert_eq!(
            run0(&module(vec![not.finish()]), 0, &[int(5)]),
            Ok(int(250))
        );
        let mut cmp = Builder::new("cmp", &[Ty::Bool, Ty::Bool], Some(Ty::Bool));
        let [a, b] = cmp.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let x = cmp.inst(Op::Bits(BitOp::Xor), Ty::Bool, &[a, b]);
        let n = cmp.inst(Op::Not, Ty::Bool, &[x]);
        cmp.ret(Some(n));
        let m = module(vec![cmp.finish()]);
        assert_eq!(
            run0(&m, 0, &[Val::Bool(true), Val::Bool(false)]),
            Ok(Val::Bool(false))
        );
        assert_eq!(
            run0(&m, 0, &[Val::Bool(true), Val::Bool(true)]),
            Ok(Val::Bool(true))
        );
    }

    #[test]
    fn test_numbers_round_trip_through_their_bits() {
        for (t, n) in [
            (U8, 255),
            (IntTy::new(8, true), -128),
            (IntTy::new(8, true), -1),
            (IntTy::new(8, true), 127),
            (I64, i64::MIN as i128),
            (I128, i128::MIN),
            (I128, -1),
            (U64, 0),
        ] {
            let v = Num::from_i128(n);
            assert_eq!(Num::from_bits(v.to_bits(t), t), v, "{} {}", t, n);
        }
        assert_eq!(Num::from_i128(-1).to_bits(U8), 255);
        assert_eq!(Num::from_bits(u128::MAX, U128), U128.max());
        assert_eq!(Num::from_bits(0x1ff, U8), Num::from_i128(255));
        assert!(!U8.holds(Num::from_i128(256)) && U8.holds(Num::from_i128(255)));
        assert!(!IntTy::new(8, true).holds(Num::from_i128(-129)));
        assert!(IntTy::new(8, true).holds(Num::from_i128(-128)));
        assert_eq!(I128.min().to_string(), i128::MIN.to_string());
        assert_eq!(U128.bytes(), 16);
    }

    #[test]
    fn test_memory_holds_arrays_and_mut_parameters() {
        // `fn bump(a: addr)`: a[1] += 1 through the address a caller passed, as `mut a: [3]u8`.
        let mut b = Builder::new("bump", &[BYTES3], None);
        let a = b.params(BlockId(0))[0];
        let one = b.int(U64, Num::from_i128(1));
        let at = b.inst(Op::Elem { stride: 1 }, BYTE, &[a, one]);
        let x = b.inst(Op::Load, Ty::Int(U8), &[at]);
        let k = b.int(U8, Num::from_i128(1));
        let y = b.inst(add(None), Ty::Int(U8), &[x, k]);
        b.inst(Op::Store, Ty::Unit, &[at, y]);
        b.ret(None);
        // `fn main() -> u8`: a slot of u16s, filled, bumped, copied, read back.
        let mut m = Builder::new("main", &[], Some(Ty::Int(U8)));
        let slot = m.inst(Op::Slot { bytes: 3 }, BYTES3, &[]);
        for i in 0..3 {
            let idx = m.int(U64, Num::from_i128(i));
            let at = m.inst(Op::Elem { stride: 1 }, BYTE, &[slot, idx]);
            let v = m.int(U8, Num::from_i128(10 * i + 1));
            m.inst(Op::Store, Ty::Unit, &[at, v]);
        }
        call(&mut m, 0, Ty::Unit, &[slot]);
        let copy = m.inst(Op::Slot { bytes: 3 }, BYTES3, &[]);
        m.inst(Op::Copy { bytes: 3 }, Ty::Unit, &[copy, slot]);
        let two = m.int(U64, Num::from_i128(2));
        let last = m.inst(Op::Elem { stride: 1 }, BYTE, &[copy, two]);
        let r2 = m.inst(Op::Load, Ty::Int(U8), &[last]);
        let one = m.int(U64, Num::from_i128(1));
        let at = m.inst(Op::Elem { stride: 1 }, BYTE, &[copy, one]);
        let r1 = m.inst(Op::Load, Ty::Int(U8), &[at]);
        let r = m.inst(add(None), Ty::Int(U8), &[r1, r2]);
        m.ret(Some(r));
        let module = module(vec![b.finish(), m.finish()]);
        assert_eq!(run0(&module, 1, &[]), Ok(int(33)));
        // A caller's memory, as `mn run --call` sets up a `mut` argument; wider elements.
        let mut memory = Memory::default();
        let a = memory.alloc(3);
        memory.store(a + 1, int(41), Ty::Int(U8)).unwrap();
        run(
            &module,
            FuncId(0),
            &[Val::Addr(a)],
            &mut memory,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(memory.load(a + 1, Ty::Int(U8)), Ok(int(42)));
        assert_eq!(memory.load(a, Ty::Bool), Ok(Val::Bool(false)));
        let w = memory.alloc(32);
        memory.store(w + 16, int(-2), Ty::Int(I128)).unwrap();
        memory.store(w, Val::Bool(true), Ty::Bool).unwrap();
        assert_eq!(memory.load(w + 16, Ty::Int(I128)), Ok(int(-2)));
        assert_eq!(memory.load(w, Ty::Bool), Ok(Val::Bool(true)));
        assert!(memory.load(w + 30, Ty::Int(U64)).is_err());
        assert_eq!(w % 16, 0);
    }

    /// Every access agrees with the shape of the memory it reaches, or the MIR is broken: an
    /// array built as one of `i64` and copied, joined or read as one of `u8` (the nightly triage's
    /// root causes 2 and 11, and the `if` and indexing forms its review found) stops compilation
    /// on every path instead of reading past a slot or reading the wrong bytes.
    #[test]
    fn test_every_access_agrees_with_the_shape_of_its_memory() {
        let mut mems = Mems::default();
        let (u8m, i64m, u64m) = (
            mems.of(Mem::Int(U8)),
            mems.of(Mem::Int(I64)),
            mems.of(Mem::Int(U64)),
        );
        let bytes4 = mems.of(Mem::Array { len: 4, elem: u8m });
        let longs4 = mems.of(Mem::Array { len: 4, elem: i64m });
        let longs2 = mems.of(Mem::Array { len: 2, elem: i64m });
        let bytes2 = mems.of(Mem::Array { len: 2, elem: u8m });
        let words2 = mems.of(Mem::Array { len: 2, elem: u64m });
        // `{0: u8, 8: [2]i64}`: a tag, and a field at offset 8.
        let record = mems.of(Mem::Record {
            size: 24,
            fields: vec![(0, u8m), (8, longs2)],
        });
        assert_eq!(mems.name(record), "{0: u8, 8: [2]i64}");
        let verified = |functions: Vec<Function>| {
            verify(&Module {
                functions,
                budget: BUDGET,
                mems: mems.clone(),
            })
        };
        let refused = |functions: Vec<Function>, why: &str| match verified(functions) {
            Err(e) => assert!(e.contains(why), "{} does not say {}", e, why),
            Ok(()) => panic!("verified, where {} breaks it", why),
        };
        let a = Ty::Addr;
        let slot = |b: &mut Builder, m: MemId| {
            let bytes = mems.size(m).unwrap_or(0);
            b.inst(Op::Slot { bytes }, a(m), &[])
        };
        // `copy 4` of a [4]i64 into a [4]u8, as root cause 11's `if` did.
        let copy = |from: MemId, bytes: u32| {
            let mut b = Builder::new("copy", &[], None);
            let to = slot(&mut b, bytes4);
            let src = slot(&mut b, from);
            b.inst(Op::Copy { bytes }, Ty::Unit, &[to, src]);
            b.ret(None);
            b.finish()
        };
        assert_eq!(verified(vec![copy(bytes4, 4)]), Ok(()));
        refused(
            vec![copy(longs4, 4)],
            "a copy of 4 bytes from memory of [4]i64 into memory of [4]u8",
        );
        refused(vec![copy(bytes4, 3)], "a copy of 3 bytes");
        // A slot takes the bytes of its shape.
        let mut b = Builder::new("slot", &[], None);
        b.inst(Op::Slot { bytes: 5 }, a(bytes4), &[]);
        b.ret(None);
        refused(vec![b.finish()], "a slot of 5 bytes for memory of [4]u8");
        // Branches that meet with arrays of [2]i64 and [2]u8, as the review's `f1` did.
        let join = |other: MemId| {
            let mut b = Builder::new("join", &[Ty::Bool], None);
            let c = b.params(BlockId(0))[0];
            let (yes, no) = (b.block(&[]), b.block(&[]));
            let met = b.block(&[a(longs2)]);
            b.branch(c, (yes, vec![]), (no, vec![]));
            for (block, m) in [(yes, longs2), (no, other)] {
                b.switch(block);
                let s = slot(&mut b, m);
                b.jump(met, vec![s]);
            }
            b.switch(met);
            b.ret(None);
            b.finish()
        };
        assert_eq!(verified(vec![join(longs2)]), Ok(()));
        refused(
            vec![join(bytes2)],
            "passes memory of [2]u8 where it takes memory of [2]i64",
        );
        // An element and its load: a [2]u64 read as `i64`, as the review's indexing did, and
        // elements taken at another stride or as another shape.
        let read = |arr: MemId, stride: u32, elem: MemId, ty: Ty| {
            let mut b = Builder::new("read", &[a(arr), Ty::Int(U64)], Some(ty));
            let [base, i] = b.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            let i = b.assume(i, Fact::Within(Num::ZERO, Num::from_i128(1)));
            let at = b.inst(Op::Elem { stride }, a(elem), &[base, i]);
            let x = b.inst(Op::Load, ty, &[at]);
            b.ret(Some(x));
            b.finish()
        };
        assert_eq!(verified(vec![read(words2, 8, u64m, Ty::Int(U64))]), Ok(()));
        refused(
            vec![read(words2, 8, u64m, Ty::Int(I64))],
            "a load of i64 from memory of u64",
        );
        refused(
            vec![read(longs2, 1, i64m, Ty::Int(I64))],
            "an element of memory of [2]i64 taken as i64 at a stride of 1",
        );
        refused(
            vec![read(longs2, 8, u64m, Ty::Int(U64))],
            "an element of memory of [2]i64 taken as u64",
        );
        // A field of a record: one at its offset, of its shape, and none elsewhere.
        let field = |offset: u128, elem: MemId| {
            let mut b = Builder::new("field", &[a(record)], None);
            let base = b.params(BlockId(0))[0];
            let k = b.int(U64, Num::new(false, offset));
            b.inst(Op::Elem { stride: 1 }, a(elem), &[base, k]);
            b.ret(None);
            b.finish()
        };
        assert_eq!(verified(vec![field(8, longs2)]), Ok(()));
        refused(
            vec![field(4, longs2)],
            "an element of memory of {0: u8, 8: [2]i64}",
        );
        refused(vec![field(8, bytes2)], "taken as [2]u8");
        // A field is taken at a stride of 1 alone, though its offset is the constant index.
        let mut b = Builder::new("stride", &[a(record)], None);
        let base = b.params(BlockId(0))[0];
        let eight = b.int(U64, Num::from_i128(8));
        b.inst(Op::Elem { stride: 8 }, a(longs2), &[base, eight]);
        b.ret(None);
        refused(
            vec![b.finish()],
            "an element of memory of {0: u8, 8: [2]i64} taken as [2]i64 at a stride of 8",
        );
        // An address of a shape the module does not hold.
        let mut b = Builder::new("unknown", &[a(MemId(99))], None);
        b.ret(None);
        refused(vec![b.finish()], "a shape the module does not hold");
        // A store takes the type its memory holds.
        let mut b = Builder::new("store", &[a(u8m)], None);
        let at = b.params(BlockId(0))[0];
        let x = b.int(I64, Num::from_i128(1));
        b.inst(Op::Store, Ty::Unit, &[at, x]);
        b.ret(None);
        refused(vec![b.finish()], "a store of i64 into memory of u8");
        // A call passes an address of the shape its callee takes.
        let callee = || {
            let mut c = Builder::new("callee", &[a(bytes4)], None);
            c.ret(None);
            c.finish()
        };
        let caller = |m: MemId| {
            let mut b = Builder::new("caller", &[], None);
            let s = slot(&mut b, m);
            call(&mut b, 0, Ty::Unit, &[s]);
            b.ret(None);
            b.finish()
        };
        assert_eq!(verified(vec![callee(), caller(bytes4)]), Ok(()));
        assert!(verified(vec![callee(), caller(longs4)]).is_err());
        // The table itself: a field past its record's bytes, or a shape held before it is added.
        let mut past = Mems::default();
        let long = past.of(Mem::Int(I64));
        past.of(Mem::Record {
            size: 4,
            fields: vec![(0, long)],
        });
        let mut ahead = Mems::default();
        ahead.of(Mem::Array {
            len: 1,
            elem: MemId(1),
        });
        ahead.of(Mem::Int(U8));
        // A shape that holds itself, whose size has no end.
        let mut itself = Mems::default();
        itself.of(Mem::Array {
            len: 1,
            elem: MemId(0),
        });
        for mems in [past, ahead, itself] {
            assert!(verify(&Module {
                functions: vec![],
                budget: BUDGET,
                mems,
            })
            .is_err());
        }
    }

    /// The order of values is checked in time and memory that grow with the function, not with
    /// its square: a chain of 20,000 diamonds, each branch reading a value of the block before
    /// it, verifies at once, where a table of every block's dominators would hold 3.6 billion
    /// entries. A verifier that grew so had the generated programs' coverage time out on CI.
    #[test]
    fn test_the_verifier_reads_a_long_function_in_linear_time() {
        let mut b = Builder::new("long", &[Ty::Bool, Ty::Int(I64)], Some(Ty::Int(I64)));
        let [c, x] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let mut value = x;
        for _ in 0..20_000 {
            let left = b.block(&[]);
            let right = b.block(&[]);
            let join = b.block(&[Ty::Int(I64)]);
            b.branch(c, (left, vec![]), (right, vec![]));
            b.switch(left);
            b.jump(join, vec![value]);
            b.switch(right);
            b.jump(join, vec![value]);
            b.switch(join);
            value = b.params(join)[0];
        }
        b.ret(Some(value));
        let started = std::time::Instant::now();
        assert!(verify(&Module {
            functions: vec![b.finish()],
            budget: BUDGET,
            mems: mems(),
        })
        .is_ok());
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    /// Every value is defined before it is used on every path (`verify_order`): an operand defined
    /// later in its block, or in a block that does not dominate its use, is refused, and so is a
    /// jump that passes one; the same values in order verify.
    #[test]
    fn test_the_verifier_refuses_a_value_used_before_it_is_defined() {
        let module = |f: Function| {
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
                mems: mems(),
            })
        };
        // `x + 1`, the constant after the addition that takes it.
        let ordered = |late: bool| {
            let mut b = Builder::new("late", &[Ty::Int(I64)], Some(Ty::Int(I64)));
            let x = b.params(BlockId(0))[0];
            let one = b.int(I64, Num::from_i128(1));
            let sum = b.inst(add(None), Ty::Int(I64), &[x, one]);
            b.ret(Some(sum));
            let mut f = b.finish();
            if late {
                f.blocks[0].insts.swap(0, 1);
            }
            f
        };
        assert!(module(ordered(false)).is_ok());
        let why = module(ordered(true)).unwrap_err();
        assert!(
            why.contains("is not defined before it on every path"),
            "{}",
            why
        );
        // `x + 1` taking itself in place of the constant: no instruction is defined before
        // itself.
        let mut itself = ordered(false);
        let sum = itself.blocks[0].insts[1];
        let (start, _) = itself.operands[sum.0 as usize];
        itself.pool[start as usize + 1] = sum;
        let why = module(itself).unwrap_err();
        assert!(
            why.contains(&format!("its operand v{} is not defined", sum.0)),
            "{}",
            why
        );
        // A value of one branch, used after the join that the other branch reaches too.
        let joined = |pass: bool| {
            let mut b = Builder::new("branch", &[Ty::Bool], Some(Ty::Int(I64)));
            let c = b.params(BlockId(0))[0];
            let then = b.block(&[]);
            let otherwise = b.block(&[]);
            let join = b.block(&[Ty::Int(I64)]);
            b.branch(c, (then, vec![]), (otherwise, vec![]));
            b.switch(then);
            let seven = b.int(I64, Num::from_i128(7));
            b.jump(join, vec![seven]);
            b.switch(otherwise);
            let eight = b.int(I64, Num::from_i128(8));
            b.jump(join, vec![if pass { eight } else { seven }]);
            b.switch(join);
            let v = b.params(join)[0];
            let r = if pass { v } else { seven };
            b.ret(Some(r));
            b.finish()
        };
        assert!(module(joined(true)).is_ok());
        let why = module(joined(false)).unwrap_err();
        assert!(
            why.contains("is not defined before it on every path"),
            "{}",
            why
        );
    }

    /// `fn f(x: i64, d: i64) -> i64`, whose effects form one chain: it guards `d != 0` and checks
    /// `x >= 0`, divides `x` exactly by the divisor the guard yields, adds 1 with a check, and
    /// branches on the sum, returning it on one side and its checked square on the other. The
    /// instructions of block 0, by name: `zero`, `nz`, `check1`, `pos`, `check2`, `q`, `one`,
    /// `s`, `neg`.
    fn chained() -> (Function, [NodeId; 9]) {
        let mut b = Builder::new("f", &[Ty::Int(I64), Ty::Int(I64)], Some(Ty::Int(I64)));
        let [x, d] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let zero = b.int(I64, Num::ZERO);
        let nz = b.inst(Op::Cmp(CmpOp::Ne), Ty::Bool, &[d, zero]);
        let t0 = b.trap_site("division-by-zero", &["division by zero"]);
        let check1 = b.guard(t0, Fact::NonZero, &[nz, d]);
        let pos = b.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[x, zero]);
        let t1 = b.trap_site("contract", &["x is negative"]);
        let check2 = b.inst(Op::Check { trap: t1 }, Ty::Unit, &[pos]);
        let q = b.inst(
            Op::Arith {
                op: ArithOp::Div,
                trap: None,
            },
            Ty::Int(I64),
            &[x, check1],
        );
        let one = b.int(I64, Num::from_i128(1));
        let t2 = b.trap_site("overflow", &["", " does not fit in `i64`"]);
        let s = b.inst(add(Some(t2)), Ty::Int(I64), &[q, one]);
        let neg = b.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[s, zero]);
        let left = b.block(&[]);
        let right = b.block(&[]);
        b.branch(neg, (left, vec![]), (right, vec![]));
        b.switch(left);
        b.ret(Some(s));
        b.switch(right);
        let t3 = b.trap_site("overflow", &["", " does not fit in `i64`"]);
        let sq = b.inst(
            Op::Arith {
                op: ArithOp::Mul,
                trap: Some(t3),
            },
            Ty::Int(I64),
            &[s, s],
        );
        b.ret(Some(sq));
        (b.finish(), [zero, nz, check1, pos, check2, q, one, s, neg])
    }

    /// The verifier holds the chain of effects whole ([`Chain`]), and refuses each way of breaking
    /// it: an effect without the latest effect, one that takes an earlier effect, forking the
    /// chain, alone or moved above the effect it skips; a value that is no effect where an effect
    /// is taken, by an instruction or by a block's effect parameter; a return before the last
    /// effect, which would leave that effect to nothing; and a partial operation, an exact
    /// division, raised above the check that guards it, or reading an earlier effect in place.
    #[test]
    fn test_the_verifier_holds_the_chain_of_effects() {
        let check = |f: Function| {
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
                mems: mems(),
            })
        };
        let (f, [_, _, check1, pos, check2, q, one, s, neg]) = chained();
        let e0 = *f.blocks[0].params.last().unwrap_or(&s);
        assert_eq!(check(f.clone()), Ok(()));
        assert_eq!(
            run0(
                &Module {
                    functions: vec![f.clone()],
                    budget: BUDGET,
                    mems: mems(),
                },
                0,
                &[int(7), int(2)]
            ),
            Ok(int(16))
        );
        // What each instruction takes last.
        let set_last = |f: &mut Function, v: NodeId, to: NodeId| {
            let (start, len) = f.operands[v.0 as usize];
            f.pool[(start + len - 1) as usize] = to;
        };
        let moved = |f: &mut Function, v: NodeId, before: NodeId| {
            let insts = &mut f.blocks[0].insts;
            insts.retain(|u| *u != v);
            let at = insts.iter().position(|u| *u == before).unwrap_or(0);
            insts.insert(at, v);
        };
        let latest = "where the latest effect is";
        let refused = |f: Function, why: &str, what: &str| {
            let got = check(f).unwrap_err();
            assert!(got.contains(why), "{}: {}", what, got);
        };
        // The checked addition without its effect: two operands where it takes one and the
        // effect.
        let mut dropped = f.clone();
        let (start, len) = dropped.operands[s.0 as usize];
        dropped.operands[s.0 as usize] = (start, len - 1);
        refused(dropped, "of type i64 on [", "dropped");
        // The second check taking the effect the first takes: a fork, alone and hoisted above it.
        let mut forked = f.clone();
        set_last(&mut forked, check2, e0);
        refused(forked.clone(), latest, "forked");
        moved(&mut forked, pos, check1);
        moved(&mut forked, check2, check1);
        refused(forked, latest, "forked and hoisted");
        // A value that is no effect where an effect is taken: the constant 1 by the addition,
        // and the sum's sign by the left block's effect parameter.
        let mut valued = f.clone();
        set_last(&mut valued, s, one);
        refused(valued, latest, "a value in an effect slot");
        let mut passed = f.clone();
        if let Term::Branch { then, .. } = &mut passed.blocks[0].term {
            if let Some(e) = then.1.last_mut() {
                *e = neg;
            }
        }
        refused(passed, latest, "a value for an effect parameter");
        // The right block returning its square after the effect before it, not after the
        // square's own check.
        // A block that takes two effect parameters, or whose last parameter is no effect.
        let mut twice = f.clone();
        let extra = twice.blocks[1].params[0];
        twice.blocks[1].params.insert(0, extra);
        if let Term::Branch { then, .. } = &mut twice.blocks[0].term {
            then.1.insert(0, e0);
        }
        refused(
            twice,
            "as its last parameter, and only there",
            "two effect parameters",
        );
        let mut swapped = f.clone();
        swapped.params = vec![Ty::Int(I64), Ty::Effect, Ty::Int(I64)];
        let d = swapped.blocks[0].params[1];
        swapped.blocks[0].params.retain(|p| *p != d);
        swapped.blocks[0].params.push(d);
        refused(
            swapped,
            "as its last parameter, and only there",
            "an effect parameter first",
        );
        let mut early = f.clone();
        let entry = *early.blocks[2].params.last().unwrap_or(&s);
        if let Term::Return(_, effect) = &mut early.blocks[2].term {
            *effect = entry;
        }
        refused(early, latest, "a return before the last effect");
        // The exact division raised above the guard of its divisor; pointed at the divisor the
        // guard takes and raised so, as a pass that hoists it would; and pointed at it in place.
        let d = f.blocks[0].params[1];
        let unguarded = "of which no guard or assumption holds nonzero";
        let mut raised = f.clone();
        moved(&mut raised, q, check1);
        refused(raised, "is not defined before it on every path", "raised");
        let mut repointed = f.clone();
        set_last(&mut repointed, q, d);
        moved(&mut repointed, q, check1);
        refused(repointed, unguarded, "pointed at the divisor and raised");
        let mut bare = f.clone();
        set_last(&mut bare, q, d);
        refused(bare, unguarded, "a divisor no guard holds");
    }

    /// Each partial operation takes the operand it is partial in from a guard or an assumption of
    /// a fact that rules its bad values out, or as a constant that is no bad value, and the
    /// verifier refuses it otherwise: a divisor that may be zero, a shift amount that may lie
    /// outside 0 to 127, an index that may be negative or past every address, and a value
    /// narrowed into a type it may not fit. A cast that widens is defined on every value, and a
    /// cast keeps what is known of the value it takes.
    #[test]
    fn test_partial_operations_take_their_operand_from_what_rules_it_out() {
        let (i, u) = (Ty::Int(I64), Ty::Int(U8));
        // `op` of type `ty` on parameters of `params`, the `k`th first assumed `fact`.
        let check = |op: Op, ty: Ty, params: &[Ty], k: usize, fact: Option<Fact>| {
            let mut b = Builder::new("partial", params, None);
            let mut ps = b.params(BlockId(0));
            if let (Some(fact), Ty::Int(_)) = (fact, params[k]) {
                ps[k] = b.assume(ps[k], fact);
            }
            b.inst(op, ty, &ps);
            b.ret(None);
            verify(&Module {
                functions: vec![b.finish()],
                budget: BUDGET,
                mems: mems(),
            })
        };
        let within =
            |lo: i128, hi: i128| Some(Fact::Within(Num::from_i128(lo), Num::from_i128(hi)));
        let div = |op| Op::Arith { op, trap: None };
        for op in [ArithOp::Div, ArithOp::Rem] {
            assert!(check(div(op), i, &[i, i], 1, None).is_err());
            assert!(check(div(op), i, &[i, i], 1, within(-1, 1)).is_err());
            assert!(check(div(op), i, &[i, i], 1, within(0, 9)).is_err());
            assert!(check(div(op), i, &[i, i], 1, within(-9, 0)).is_err());
            assert_eq!(check(div(op), i, &[i, i], 1, within(1, 9)), Ok(()));
            assert_eq!(check(div(op), i, &[i, i], 1, within(-9, -1)), Ok(()));
            assert_eq!(check(div(op), i, &[i, i], 1, Some(Fact::NonZero)), Ok(()));
        }
        // A constant divisor: not zero, or zero.
        for (n, ok) in [(3, true), (0, false)] {
            let mut b = Builder::new("constant", &[i], None);
            let x = b.params(BlockId(0))[0];
            let k = b.int(I64, Num::from_i128(n));
            b.inst(div(ArithOp::Div), i, &[x, k]);
            b.ret(None);
            let verified = verify(&Module {
                functions: vec![b.finish()],
                budget: BUDGET,
                mems: mems(),
            });
            assert_eq!(verified.is_ok(), ok, "{}", n);
        }
        let shl = Op::Shl { trap: TrapId(0) };
        for op in [Op::Shr, shl] {
            let checked = |fact| {
                let mut b = Builder::new("shift", &[i, i], None);
                b.trap_site("narrowing", &["", " does not fit"]);
                let [x, k] = b.params(BlockId(0))[..] else {
                    panic!("two parameters")
                };
                let k = match fact {
                    Some(fact) => b.assume(k, fact),
                    None => k,
                };
                b.inst(op.clone(), i, &[x, k]);
                b.ret(None);
                verify(&Module {
                    functions: vec![b.finish()],
                    budget: BUDGET,
                    mems: mems(),
                })
            };
            assert!(checked(None).is_err());
            assert!(checked(within(-1, 127)).is_err());
            assert!(checked(within(0, 128)).is_err());
            assert!(checked(Some(Fact::NonZero)).is_err());
            assert_eq!(checked(within(0, 127)), Ok(()));
        }
        let elem = Op::Elem { stride: 1 };
        let past = Some(Fact::Within(
            Num::ZERO,
            Num::new(false, u128::from(u64::MAX) + 1),
        ));
        assert!(check(elem.clone(), BYTE, &[BYTES3, i], 1, None).is_err());
        assert!(check(elem.clone(), BYTE, &[BYTES3, i], 1, within(-1, 5)).is_err());
        assert!(check(elem.clone(), BYTE, &[BYTES3, i], 1, past).is_err());
        assert_eq!(check(elem, BYTE, &[BYTES3, i], 1, within(0, 5)), Ok(()));
        // An i64 into a u8, and a u8 into an i64.
        assert!(check(Op::Cast, u, &[i], 0, None).is_err());
        assert!(check(Op::Cast, u, &[i], 0, within(-1, 255)).is_err());
        assert!(check(Op::Cast, u, &[i], 0, within(0, 256)).is_err());
        assert_eq!(check(Op::Cast, u, &[i], 0, within(0, 255)), Ok(()));
        assert_eq!(check(Op::Cast, i, &[u], 0, None), Ok(()));
        // An i64 into a u64, which holds its largest value and not its smallest, and a u8 into
        // an i8, which holds its smallest and not its largest.
        let (u64t, i8t) = (Ty::Int(IntTy::new(64, false)), Ty::Int(IntTy::new(8, true)));
        assert!(check(Op::Cast, u64t, &[i], 0, None).is_err());
        assert!(check(Op::Cast, i8t, &[u], 0, None).is_err());
        assert_eq!(check(Op::Cast, i8t, &[u], 0, within(0, 127)), Ok(()));
        // A divisor through a cast that keeps it: assumed not zero as a u8, divided as an i64.
        let mut b = Builder::new("through", &[i, u], None);
        let [x, d] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let d = b.assume(d, Fact::NonZero);
        let d = b.inst(Op::Cast, i, &[d]);
        b.inst(div(ArithOp::Div), i, &[x, d]);
        b.ret(None);
        let f = b.finish();
        assert_eq!(f.fact(d), Some(Fact::NonZero));
        assert_eq!(
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
                mems: mems(),
            }),
            Ok(())
        );
    }

    /// A block is entered with the same counts open on every path into it (`verify_frames`): a
    /// count on one path and its call after the join, and a count before a branch with its call on
    /// one side only, each enter the join with the count open on one path and not on the other. A
    /// count before the branch and its call after the join verify.
    #[test]
    fn test_counts_open_on_some_paths_into_a_block_are_refused() {
        // `fn leaf()`, which a caller counts and calls.
        let mut l = Builder::new("leaf", &[], None);
        l.ret(None);
        let mut leaf = l.finish();
        leaf.frame = 64;
        // A count on one path into the join, and its call after the join: the call takes a count
        // the other path never defined.
        let shaped = |count_first: bool, call_in_then: bool| {
            let mut b = Builder::new("joined", &[Ty::Bool], None);
            let c = b.params(BlockId(0))[0];
            let trap = Some(b.trap_site("stack-overflow", &["the call stack ran out"]));
            let enter = Op::Enter {
                callee: FuncId(0),
                trap,
            };
            let early = count_first.then(|| b.inst(enter.clone(), Ty::Unit, &[]));
            let then = b.block(&[]);
            let otherwise = b.block(&[]);
            let join = b.block(&[]);
            b.branch(c, (then, vec![]), (otherwise, vec![]));
            b.switch(then);
            if let (Some(_), true) = (early, call_in_then) {
                b.inst(Op::Call(FuncId(0)), Ty::Unit, &[]);
            }
            b.jump(join, vec![]);
            b.switch(otherwise);
            let late = early.or_else(|| Some(b.inst(enter, Ty::Unit, &[])));
            b.jump(join, vec![]);
            b.switch(join);
            match (late, call_in_then) {
                (Some(_), false) => {
                    b.inst(Op::Call(FuncId(0)), Ty::Unit, &[]);
                    b.ret(None);
                }
                // A join that only traps returns nothing, so that the join itself is what fails.
                (_, true) | (None, false) => {
                    let ends = b.trap_site("unreachable", &["no value"]);
                    b.trap(ends);
                }
            }
            Module {
                functions: vec![leaf.clone(), b.finish()],
                budget: BUDGET,
                mems: mems(),
            }
        };
        // The count only in `otherwise`, the call after the join: the join is entered with the
        // count open on one path and on the other none.
        let why = verify(&shaped(false, false)).unwrap_err();
        assert!(
            why.contains("is entered with different counts of calls open"),
            "{}",
            why
        );
        // The count before the branch, the call only in `then`: the join, which traps, is entered
        // with the count open on one path and closed on the other.
        let why = verify(&shaped(true, true)).unwrap_err();
        assert!(
            why.contains("is entered with different counts of calls open"),
            "{}",
            why
        );
        // The count before the branch and the call after the join verify.
        verify(&shaped(true, false)).unwrap();
    }

    /// A recursion without end stops where its counts reach the budget, at the `Enter` whose frame
    /// would pass it, as a trap at that call: frames of 64 bytes fill a budget of 2^20 with 16,384
    /// calls, and the 16,385th traps. Calls one after another give their bytes back: a function
    /// that makes two calls of a frame of more than half the budget runs. A run is also bounded
    /// in steps.
    #[test]
    fn test_runs_stop_at_their_limits() {
        // `fn down(n: i64) -> i64 = down(n)`: recursion without end.
        let mut b = Builder::new("down", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let n = b.params(BlockId(0))[0];
        b.at(3, 5);
        let r = call(&mut b, 0, Ty::Int(I64), &[n]);
        b.ret(Some(r));
        let mut down = b.finish();
        down.frame = 64;
        let m = module(vec![down]);
        assert_eq!(
            run0(&m, 0, &[int(1)]),
            Err(Stop::Trap(
                "trap[stack-overflow] at 3:5: the call stack ran out".to_string()
            ))
        );
        // `fn count(n: i64) { if n == 0 { return }  count(n - 1) }`, of a frame of 64 bytes, run as
        // the entry, which counts nothing: `count(k)` makes k calls, which fit while k * 64 is
        // within the budget.
        let mut b = Builder::new("count", &[Ty::Int(I64)], None);
        let n = b.params(BlockId(0))[0];
        let zero = b.int(I64, Num::ZERO);
        let done = b.inst(Op::Cmp(CmpOp::Eq), Ty::Bool, &[n, zero]);
        let stop = b.block(&[]);
        let more = b.block(&[]);
        b.branch(done, (stop, vec![]), (more, vec![]));
        b.switch(stop);
        b.ret(None);
        b.switch(more);
        let one = b.int(I64, Num::from_i128(1));
        let less = b.inst(
            Op::Arith {
                op: ArithOp::Sub,
                trap: None,
            },
            Ty::Int(I64),
            &[n, one],
        );
        call(&mut b, 0, Ty::Unit, &[less]);
        b.ret(None);
        let mut count = b.finish();
        count.frame = 64;
        let m = module(vec![count.clone()]);
        let fits = (BUDGET / 64) as i128;
        assert_eq!(run0(&m, 0, &[int(fits)]), Ok(Val::Unit));
        assert!(
            matches!(run0(&m, 0, &[int(fits + 1)]), Err(Stop::Trap(t)) if t.starts_with("trap[stack-overflow]"))
        );
        // Two calls of a frame of more than half the budget, one after the other.
        let mut big = Builder::new("big", &[], None);
        big.ret(None);
        let mut big = big.finish();
        big.frame = BUDGET / 2 + 1;
        let mut b = Builder::new("twice", &[], None);
        for _ in 0..2 {
            call(&mut b, 0, Ty::Unit, &[]);
        }
        b.ret(None);
        let m = module(vec![big.clone(), b.finish()]);
        assert_eq!(run0(&m, 1, &[]), Ok(Val::Unit));
        // A call that has returned gives its frame back, and no more: the entry calls `after(k)`,
        // of 1,024 bytes, which calls a function of 100,000 that returns, then `count(k)`, whose
        // k + 1 frames of 64 fit the 1,047,552 bytes left up to k = 16,367.
        let mut small = big;
        small.frame = 100_000;
        let mut a = Builder::new("after", &[Ty::Int(I64)], None);
        let k = a.params(BlockId(0))[0];
        call(&mut a, 1, Ty::Unit, &[]);
        call(&mut a, 0, Ty::Unit, &[k]);
        a.ret(None);
        let mut after = a.finish();
        after.frame = 1024;
        let mut e = Builder::new("entry", &[Ty::Int(I64)], None);
        let k = e.params(BlockId(0))[0];
        call(&mut e, 2, Ty::Unit, &[k]);
        e.ret(None);
        let m = module(vec![count, small, after, e.finish()]);
        assert_eq!(run0(&m, 3, &[int(16_367)]), Ok(Val::Unit));
        assert!(
            matches!(run0(&m, 3, &[int(16_368)]), Err(Stop::Trap(t)) if t.starts_with("trap[stack-overflow]"))
        );
        let m = module(vec![sum()]);
        let limits = Limits { max_steps: 100 };
        assert_eq!(
            run(&m, FuncId(0), &[int(1000)], &mut Memory::default(), limits),
            Err(Stop::OutOfSteps)
        );
        assert_eq!(
            run(&m, FuncId(0), &[int(10)], &mut Memory::default(), limits),
            Ok(int(45))
        );
    }

    #[test]
    fn test_the_verifier_rejects_malformed_functions() {
        let wrong = |f: Function| {
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
                mems: mems(),
            })
            .unwrap_err()
        };
        // A jump with the wrong arguments.
        let mut f = sum();
        f.blocks[0].term = Term::Jump(BlockId(1), vec![]);
        assert!(wrong(f).contains("wrong arguments"));
        let mut f = sum();
        f.blocks[0].term = Term::Jump(BlockId(9), vec![]);
        assert!(wrong(f).contains("does not exist"));
        // A return of the wrong type.
        let mut f = sum();
        let effect = f.blocks[3].term.clone();
        let Term::Return(_, effect) = effect else {
            panic!("sum's block 3 returns")
        };
        f.blocks[3].term = Term::Return(None, effect);
        assert!(wrong(f).contains("returns the wrong type"));
        // An addition of an i64 and a bool.
        let mut b = Builder::new("bad", &[Ty::Int(I64)], None);
        let x = b.params(BlockId(0))[0];
        let t = b.inst(Op::Bool(true), Ty::Bool, &[]);
        b.inst(add(None), Ty::Int(I64), &[x, t]);
        assert!(wrong(b.finish()).contains("Arith"));
        // A call of a function that does not exist, and a trap site that does not.
        let mut b = Builder::new("call", &[], None);
        call(&mut b, 7, Ty::Unit, &[]);
        assert!(wrong(b.finish()).contains("does not exist"));
        let mut b = Builder::new("trap", &[], None);
        b.trap(TrapId(0));
        assert!(wrong(b.finish()).contains("site that does not exist"));
        // A branch on an integer.
        let mut f = sum();
        if let Term::Branch { cond, .. } = &mut f.blocks[1].term {
            *cond = NodeId(0);
        }
        assert!(wrong(f).contains("not a bool"));
        // A check whose site has no room for the values it reports.
        let mut b = Builder::new("check", &[Ty::Bool, Ty::Int(I64)], None);
        let [c, x] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let site = b.trap_site("narrowing", &["no value"]);
        b.inst(Op::Check { trap: site }, Ty::Unit, &[c, x]);
        assert!(wrong(b.finish()).contains("Check"));
        // An entry block that does not take the function's parameters.
        let mut f = sum();
        f.params = vec![Ty::Bool];
        assert!(wrong(f).contains("entry block"));
    }

    /// A function of one block whose instruction is `op` of type `ty` on `args`, built from
    /// parameters of `params`; `traps` are the sites' texts. An integer operand the instruction
    /// is partial in is first assumed to hold what it needs ([`Function::partial_in`]).
    fn one(op: Op, ty: Ty, params: &[Ty], args: &[usize], traps: &[&[&str]]) -> Function {
        let build = |need: Option<(usize, Fact)>| {
            let mut b = Builder::new("one", params, None);
            let ps = b.params(BlockId(0));
            for t in traps {
                b.trap_site("narrowing", t);
            }
            let mut operands: Vec<NodeId> = args.iter().map(|i| ps[*i]).collect();
            if let Some((k, fact)) = need {
                if let (Some(o), Some(&Ty::Int(_))) =
                    (operands.get(k).copied(), args.get(k).map(|i| &params[*i]))
                {
                    operands[k] = b.assume(o, fact);
                }
            }
            b.inst(op.clone(), ty, &operands);
            b.ret(None);
            b.finish()
        };
        let plain = build(None);
        let need = plain.blocks[0]
            .insts
            .last()
            .and_then(|v| plain.partial_in(*v));
        match need {
            Some((k, need)) => build(Some((k, need.fact()))),
            None => plain,
        }
    }

    #[test]
    fn test_the_verifier_checks_each_instruction_against_its_operation() {
        let check = |f: Function| {
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
                mems: mems(),
            })
        };
        let add = |trap| Op::Arith {
            op: ArithOp::Add,
            trap,
        };
        let i = Ty::Int(I64);
        let u = Ty::Int(U8);
        // Well formed.
        for f in [
            one(add(None), i, &[i, i], &[0, 1], &[]),
            one(add(Some(TrapId(0))), i, &[i, i], &[0, 1], &[&["", "x"]]),
            one(
                Op::Check { trap: TrapId(0) },
                Ty::Unit,
                &[Ty::Bool, i, u],
                &[0, 1, 2],
                &[&["", "", ""]],
            ),
            one(Op::Elem { stride: 1 }, BYTE, &[BYTES3, u], &[0, 1], &[]),
            one(Op::Store, Ty::Unit, &[FLAG, Ty::Bool], &[0, 1], &[]),
            one(
                Op::Cmp(CmpOp::Eq),
                Ty::Bool,
                &[Ty::Bool, Ty::Bool],
                &[0, 1],
                &[],
            ),
            one(
                Op::Shl { trap: TrapId(0) },
                u,
                &[i, u],
                &[0, 1],
                &[&["", "x"]],
            ),
        ] {
            assert_eq!(check(f), Ok(()));
        }
        // Each wrong in one way: the result, the count of operands, an operand, the site.
        for f in [
            one(add(None), Ty::Bool, &[i, i], &[0, 1], &[]),
            one(add(None), i, &[i], &[0], &[]),
            one(add(None), i, &[i, u], &[0, 1], &[]),
            one(add(Some(TrapId(0))), i, &[i, i], &[0, 1], &[&["no room"]]),
            one(
                Op::Check { trap: TrapId(0) },
                Ty::Unit,
                &[Ty::Bool, i],
                &[0, 1],
                &[&["one piece"]],
            ),
            one(
                Op::Check { trap: TrapId(0) },
                Ty::Unit,
                &[i],
                &[0],
                &[&["x"]],
            ),
            one(
                Op::Elem { stride: 1 },
                BYTES3,
                &[BYTES3, Ty::Bool],
                &[0, 1],
                &[],
            ),
            one(Op::Load, BYTES3, &[BYTES3], &[0], &[]),
            one(Op::Store, Ty::Unit, &[i, i], &[0, 1], &[]),
            one(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, u], &[0, 1], &[]),
            one(
                Op::Cmp(CmpOp::Lt),
                Ty::Bool,
                &[BYTES3, BYTES3],
                &[0, 1],
                &[],
            ),
            one(
                Op::Shl { trap: TrapId(0) },
                u,
                &[Ty::Bool, u],
                &[0, 1],
                &[&["", "x"]],
            ),
            one(Op::Shr, u, &[i, u], &[0, 1], &[]),
            one(Op::Cast, u, &[Ty::Bool], &[0], &[]),
            one(Op::Not, BYTES3, &[BYTES3], &[0], &[]),
            one(Op::Bits(BitOp::And), u, &[u, i], &[0, 1], &[]),
            one(Op::Copy { bytes: 1 }, Ty::Unit, &[BYTES3, u], &[0, 1], &[]),
            one(Op::Int(1), Ty::Bool, &[], &[], &[]),
            one(Op::Bool(true), u, &[], &[], &[]),
            one(Op::Slot { bytes: 1 }, u, &[], &[], &[]),
            one(Op::Neg { trap: None }, u, &[i], &[0], &[]),
            one(Op::Param, u, &[], &[], &[]),
        ] {
            assert!(check(f).is_err());
        }
        // A call takes its callee's parameters and gives its result.
        let mut callee = Builder::new("callee", &[u], Some(i));
        let p = callee.params(BlockId(0))[0];
        let w = callee.inst(Op::Cast, i, &[p]);
        callee.ret(Some(w));
        let callee = callee.finish();
        let caller = |ty: Ty, param: Ty| {
            let mut b = Builder::new("caller", &[param], None);
            let p = b.params(BlockId(0))[0];
            call(&mut b, 0, ty, &[p]);
            b.ret(None);
            Module {
                functions: vec![callee.clone(), b.finish()],
                budget: BUDGET,
                mems: mems(),
            }
        };
        assert_eq!(verify(&caller(i, u)), Ok(()));
        assert!(verify(&caller(u, u)).is_err());
        assert!(verify(&caller(i, i)).is_err());
        // An operand one past the last value.
        let mut f = sum();
        let last = NodeId(f.ops.len() as u32);
        f.pool[0] = last;
        assert!(verify(&Module {
            functions: vec![f],
            budget: BUDGET,
            mems: mems(),
        })
        .unwrap_err()
        .contains("does not exist"));
    }

    /// MIR keeps an address only in a value, never in memory: the verifier refuses a store of an
    /// address and a load that gives one, beside the store and the load of an integer it admits.
    /// Native code lays the slots never alive together over the same bytes by what this holds,
    /// that a slot is reached only through the values derived from its own
    /// (`spec/mir-and-codegen.md`, "Slots never alive together share bytes").
    #[test]
    fn test_no_address_is_stored_in_memory_or_loaded_from_it() {
        let u8t = Ty::Int(U8);
        let function = |stored: Ty, loaded: Ty| {
            let mut b = Builder::new("f", &[u8t], Some(u8t));
            let x = b.params(BlockId(0))[0];
            let s = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
            let value = if stored == Ty::Addr { s } else { x };
            b.inst(Op::Store, Ty::Unit, &[s, value]);
            b.inst(Op::Load, loaded, &[s]);
            b.ret(Some(x));
            Module {
                functions: vec![b.finish()],
                budget: BUDGET,
            }
        };
        verify(&function(u8t, u8t)).unwrap();
        let stored = verify(&function(Ty::Addr, u8t)).unwrap_err();
        assert!(stored.contains("Store"), "{stored}");
        let loaded = verify(&function(u8t, Ty::Addr)).unwrap_err();
        assert!(loaded.contains("Load"), "{loaded}");
    }

    /// No operation but those of addresses takes an address or gives one, whatever its kind: a
    /// slot and an element give one; an element, a load, a store and a copy take one where they
    /// reach memory; a count takes the arguments of its call as they were evaluated, an address
    /// among them; a call passes one only to a parameter that is one; and no function returns
    /// one, so no address outlives its slot. Each kind of operation is tried with an address in
    /// each place that is not one of those, and in its result, and refused. Native code lays the
    /// slots never alive together over the same bytes by what this holds, that a slot is reached
    /// only through the values derived from its own (`spec/mir-and-codegen.md`, "Slots never
    /// alive together share bytes"); a `cast` or a `guard` that took an address would break it.
    #[test]
    fn test_only_the_operations_of_addresses_take_or_give_one() {
        let (u, i, a, b) = (Ty::Int(U8), Ty::Int(I64), Ty::Addr, Ty::Bool);
        // Which operand of an operation may be an address, and whether its result may be one. The
        // matches name every operation, so that a new one is placed here before it builds.
        let takes = |op: &Op, k: usize| match op {
            Op::Elem { .. } | Op::Load | Op::Store => k == 0,
            Op::Copy { .. } | Op::Enter { .. } => true,
            Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Shl { .. }
            | Op::Shr
            | Op::Cmp(_)
            | Op::Cast
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Slot { .. }
            | Op::Call(_) => false,
        };
        let gives = |op: &Op| match op {
            Op::Slot { .. } | Op::Elem { .. } => true,
            Op::Param
            | Op::Int(_)
            | Op::Bool(_)
            | Op::Arith { .. }
            | Op::Neg { .. }
            | Op::Bits(_)
            | Op::Not
            | Op::Shl { .. }
            | Op::Shr
            | Op::Cmp(_)
            | Op::Cast
            | Op::Check { .. }
            | Op::Guard { .. }
            | Op::Assume(_)
            | Op::Load
            | Op::Store
            | Op::Copy { .. }
            | Op::Call(_)
            | Op::Enter { .. } => false,
        };
        let check = |f: Function| {
            verify(&Module {
                functions: vec![f],
                budget: BUDGET,
            })
        };
        let add = |trap| Op::Arith {
            op: ArithOp::Add,
            trap,
        };
        let x: &[&str] = &["", "x"];
        // Each kind built by `one`: the operation, its result, its parameters (one an operand),
        // and its trap sites. Each well formed as it stands.
        type Sample<'a> = (Op, Ty, Vec<Ty>, Vec<&'a [&'a str]>);
        let samples: Vec<Sample> = vec![
            (Op::Int(1), i, vec![], vec![]),
            (Op::Bool(true), b, vec![], vec![]),
            (add(None), i, vec![i, i], vec![]),
            (add(Some(TrapId(0))), i, vec![i, i], vec![x]),
            (Op::Neg { trap: None }, i, vec![i], vec![]),
            (Op::Bits(BitOp::And), u, vec![u, u], vec![]),
            (Op::Not, u, vec![u], vec![]),
            (Op::Shl { trap: TrapId(0) }, u, vec![u, u], vec![x]),
            (Op::Shr, u, vec![u, u], vec![]),
            (Op::Cmp(CmpOp::Eq), b, vec![u, u], vec![]),
            (Op::Cast, i, vec![u], vec![]),
            (Op::Check { trap: TrapId(0) }, Ty::Unit, vec![b, u], vec![x]),
            (Op::Slot { bytes: 1 }, a, vec![], vec![]),
            (Op::Elem { stride: 1 }, a, vec![a, u], vec![]),
            (Op::Load, u, vec![a], vec![]),
            (Op::Store, Ty::Unit, vec![a, u], vec![]),
            (Op::Copy { bytes: 1 }, Ty::Unit, vec![a, a], vec![]),
        ];
        for (op, ty, params, traps) in &samples {
            let args: Vec<usize> = (0..params.len()).collect();
            assert_eq!(
                check(one(op.clone(), *ty, params, &args, traps)),
                Ok(()),
                "{op:?}"
            );
            for k in (0..params.len()).filter(|k| !takes(op, *k)) {
                let mut wrong = params.clone();
                wrong[k] = a;
                assert!(
                    check(one(op.clone(), *ty, &wrong, &args, traps)).is_err(),
                    "{op:?} takes an address as its operand {k}"
                );
            }
            if !gives(op) {
                assert!(
                    check(one(op.clone(), a, params, &args, traps)).is_err(),
                    "{op:?} gives an address"
                );
            }
        }
        // A guard and an assumption yield the integer they take, each held in a function of its
        // own, so that one's refusal of an address cannot stand for the other's (the review of
        // pull request 183: a guard that took one passed while beside an assumption).
        for guarded in [u, a] {
            let mut f = Builder::new("g", &[b, guarded], None);
            let [c, v] = f.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            let site = f.trap_site("narrowing", &["x does not fit"]);
            let fact = f.fact_site(Fact::Within(Num::ZERO, Num::from_i128(3)));
            f.inst(Op::Guard { trap: site, fact }, guarded, &[c, v]);
            f.ret(None);
            assert_eq!(
                check(f.finish()).is_ok(),
                guarded == u,
                "a guard of {guarded}"
            );
            let mut f = Builder::new("g", &[guarded], None);
            let v = f.params(BlockId(0))[0];
            f.assume(v, Fact::Within(Num::ZERO, Num::from_i128(3)));
            f.ret(None);
            assert_eq!(
                check(f.finish()).is_ok(),
                guarded == u,
                "an assumption of {guarded}"
            );
        }
        // A count takes what it counts, an address among them; a call passes an address only to a
        // parameter that is one.
        let callee = |param: Ty| {
            let mut g = Builder::new("callee", &[param], None);
            g.ret(None);
            g.finish()
        };
        for (param, arg, ok) in [(a, a, true), (u, a, false), (a, u, false)] {
            let mut f = Builder::new("caller", &[arg], None);
            let p = f.params(BlockId(0))[0];
            call(&mut f, 0, Ty::Unit, &[p]);
            f.ret(None);
            let verified = verify(&Module {
                functions: vec![callee(param), f.finish()],
                budget: BUDGET,
            });
            assert_eq!(verified.is_ok(), ok, "{param} given {arg}: {verified:?}");
        }
        // No function returns an address, so a call never gives one.
        let mut f = Builder::new("leak", &[], Some(a));
        let s = f.inst(Op::Slot { bytes: 1 }, a, &[]);
        f.ret(Some(s));
        let leaked = check(f.finish()).unwrap_err();
        assert!(leaked.contains("returns an address"), "{leaked}");
    }

    /// A slot read before it is written holds the memory's fill, 0 unless another is asked
    /// for, and a copy of memory starts runs of its own; a run counts what it executed.
    #[test]
    fn test_unwritten_memory_holds_its_fill_and_runs_count_their_steps() {
        // `fn peek() -> u16`: a slot's first two bytes, never written.
        let mut b = Builder::new("peek", &[], Some(Ty::Int(IntTy::new(16, false))));
        let s = b.inst(Op::Slot { bytes: 2 }, WORD, &[]);
        let v = b.inst(Op::Load, Ty::Int(IntTy::new(16, false)), &[s]);
        b.ret(Some(v));
        let m = module(vec![b.finish()]);
        let peek =
            |memory: &mut Memory| run_counting(&m, FuncId(0), &[], memory, Limits::default());
        assert_eq!(peek(&mut Memory::default()), (Ok(int(0)), 2));
        let mut poisoned = Memory::filled(0xA5);
        let at = poisoned.alloc(1);
        let copy = poisoned.clone();
        assert_eq!(peek(&mut poisoned), (Ok(int(0xA5A5)), 2));
        assert_eq!(copy.load(at, Ty::Int(U8)), Ok(int(0xA5)));
        assert_eq!(copy.used(), 1);
    }

    /// An index no check kept inside its array, as a mutant that removes the check makes, is
    /// broken MIR wherever its access leaves the slot: into the gap before the next slot, across
    /// a slot's end, past the last slot or beyond every address, and never a panic.
    #[test]
    fn test_accesses_outside_their_slot_are_broken_mir() {
        // `fn at(i) -> u16`: a u16 at byte `i` of a 3-byte slot, with a 4-byte slot after it.
        let at = |index: i128, signed: bool| {
            let t = IntTy::new(64, signed);
            let u16 = IntTy::new(16, false);
            let mut b = Builder::new("at", &[Ty::Int(t)], Some(Ty::Int(u16)));
            let i = b.params(BlockId(0))[0];
            let s = b.inst(Op::Slot { bytes: 3 }, BYTES3, &[]);
            let _next = b.inst(Op::Slot { bytes: 4 }, BYTES4, &[]);
            let inside = Fact::Within(Num::ZERO, Num::new(false, u128::from(u64::MAX)));
            let i = b.assume(i, inside);
            let e = b.inst(Op::Elem { stride: 1 }, BYTE, &[s, i]);
            let v = b.inst(Op::Load, Ty::Int(u16), &[e]);
            b.ret(Some(v));
            // A `u16` read from memory of bytes is no access the verifier admits: the run shows
            // what the interpreter makes of MIR that breaks it, as a mutant may.
            let unverified = Module {
                functions: vec![b.finish()],
                budget: BUDGET,
                mems: mems(),
            };
            run(
                &unverified,
                FuncId(0),
                &[int(index)],
                &mut Memory::filled(7),
                Limits::default(),
            )
        };
        let broken = |r: Result<Val, Stop>| matches!(r, Err(Stop::Broken(why)) if why.contains("outside its slot"));
        assert_eq!(at(1, false), Ok(int(0x0707)));
        assert!(
            broken(at(2, false)),
            "the load's second byte leaves the slot"
        );
        assert!(broken(at(5, false)), "the gap between the slots");
        assert!(
            broken(at(19, false)),
            "the next slot's last byte and past it"
        );
        assert!(
            broken(at(i128::from(u64::MAX), false)),
            "the address overflows"
        );
        assert!(matches!(at(-1, true), Err(Stop::Broken(_))));
        // An address inside another slot reads it: slots are told apart by address alone.
        assert_eq!(at(16, false), Ok(int(0x0707)));
        // A slot released on return is gone with its frame.
        let mut memory = Memory::filled(0);
        let kept = memory.alloc(2);
        let call = memory.mark();
        memory.alloc(2);
        memory.release(call);
        assert_eq!(memory.load(kept, Ty::Int(U8)), Ok(int(0)));
        assert!(memory.load(16, Ty::Int(U8)).is_err());
        // A slot of no bytes made just before a call begins where the call's memory does, and
        // stays with its frame when the call returns.
        let mut memory = Memory::filled(0);
        memory.alloc(2);
        let empty = memory.alloc(0);
        let call = memory.mark();
        let callee = memory.alloc(2);
        assert_eq!(callee, empty);
        memory.release(call);
        assert_eq!(memory.copy(empty, empty, 0), Ok(()));
        assert!(memory.load(callee, Ty::Int(U8)).is_err());
    }

    #[test]
    fn test_runs_count_steps_and_give_back_memory() {
        // Three instructions: a slot, a constant and its store.
        let mut b = Builder::new("three", &[], None);
        let s = b.inst(Op::Slot { bytes: 8 }, LONG, &[]);
        let k = b.int(I64, Num::from_i128(7));
        b.inst(Op::Store, Ty::Unit, &[s, k]);
        b.ret(None);
        let m = module(vec![b.finish()]);
        let limited = |steps| Limits { max_steps: steps };
        let mut memory = Memory::default();
        let before = memory.used();
        assert_eq!(
            run(&m, FuncId(0), &[], &mut memory, limited(3)),
            Ok(Val::Unit)
        );
        assert_eq!(memory.used(), before);
        assert_eq!(
            run(&m, FuncId(0), &[], &mut Memory::default(), limited(2)),
            Err(Stop::OutOfSteps)
        );
    }

    #[test]
    fn test_unchecked_shift_amounts_are_broken_mir() {
        let shr = |amount: i128| {
            let mut b = Builder::new("shr", &[Ty::Int(I128), Ty::Int(I128)], Some(Ty::Int(I128)));
            let [x, k] = b.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            let k = b.assume(k, Fact::Within(Num::ZERO, Num::new(false, 127)));
            let r = b.inst(Op::Shr, Ty::Int(I128), &[x, k]);
            b.ret(Some(r));
            run0(&module(vec![b.finish()]), 0, &[int(8), int(amount)])
        };
        assert_eq!(shr(127), Ok(int(0)));
        assert!(matches!(shr(-1), Err(Stop::Broken(_))));
        assert!(matches!(shr(128), Err(Stop::Broken(_))));
        // A negative constant prints as its value.
        let mut b = Builder::new("k", &[], Some(Ty::Int(I64)));
        let k = b.int(I64, Num::from_i128(-5));
        b.ret(Some(k));
        assert!(b.finish().to_string().contains("= int -5 @"));
    }
}

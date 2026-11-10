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
    /// The address of memory: a stack slot, an element of one, or what a caller passed.
    Addr,
    /// What an instruction without a result has.
    Unit,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int(t) => write!(f, "{}", t),
            Ty::Bool => write!(f, "bool"),
            Ty::Addr => write!(f, "addr"),
            Ty::Unit => write!(f, "unit"),
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
    /// A new stack slot of `bytes`, aligned to 16, its address the value.
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
    /// Calls a function of the module; the value is its result, or unit.
    Call(FuncId),
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
    Return(Option<NodeId>),
    Trap(TrapId),
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
    pub ops: Vec<Op>,
    pub tys: Vec<Ty>,
    /// Each value's operands, a range of `pool`.
    pub operands: Vec<(u32, u32)>,
    pub locs: Vec<Loc>,
    pub pool: Vec<NodeId>,
    pub blocks: Vec<Block>,
    pub traps: Vec<TrapSite>,
}

impl Function {
    pub fn args(&self, v: NodeId) -> &[NodeId] {
        let (start, len) = self.operands[v.0 as usize];
        &self.pool[start as usize..(start + len) as usize]
    }

    pub fn ty(&self, v: NodeId) -> Ty {
        self.tys[v.0 as usize]
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
                _ => format!("int {}", bits),
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
            Op::Slot { bytes } => format!("slot {}", bytes),
            Op::Elem { stride } => format!("elem {}", stride),
            Op::Load => "load".into(),
            Op::Store => "store".into(),
            Op::Copy { bytes } => format!("copy {}", bytes),
            Op::Call(c) => format!("call f{}", c.0),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Module {
    pub functions: Vec<Function>,
}

/// Builds a function one block at a time.
pub struct Builder {
    f: Function,
    current: BlockId,
    loc: Loc,
}

impl Builder {
    /// A function whose entry block takes `params`.
    pub fn new(name: &str, params: &[Ty], result: Option<Ty>) -> Builder {
        Builder::declared_at(name, params, result, Loc::default())
    }

    /// A function whose entry block takes `params`, declared at `decl`: its parameters carry
    /// that location, which a debugger names as the function's.
    pub fn declared_at(name: &str, params: &[Ty], result: Option<Ty>, decl: Loc) -> Builder {
        let mut b = Builder {
            f: Function {
                name: name.to_string(),
                params: params.to_vec(),
                result,
                ..Function::default()
            },
            current: BlockId(0),
            loc: decl,
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

    /// A new block taking `params`, ended by a trap until it is given its terminator.
    pub fn block(&mut self, params: &[Ty]) -> BlockId {
        let id = BlockId(self.f.blocks.len() as u32);
        let params = params
            .iter()
            .map(|t| self.value(Op::Param, *t, &[]))
            .collect();
        self.f.blocks.push(Block {
            params,
            insts: Vec::new(),
            term: Term::Return(None),
        });
        id
    }

    pub fn params(&self, b: BlockId) -> Vec<NodeId> {
        self.f.block(b).params.clone()
    }

    /// Builds into block `b` from now on.
    pub fn switch(&mut self, b: BlockId) {
        self.current = b;
    }

    pub fn current(&self) -> BlockId {
        self.current
    }

    /// Appends an instruction to the current block.
    pub fn inst(&mut self, op: Op, ty: Ty, args: &[NodeId]) -> NodeId {
        let v = self.value(op, ty, args);
        self.f.blocks[self.current.0 as usize].insts.push(v);
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

    /// Ends the current block.
    pub fn end(&mut self, term: Term) {
        self.f.blocks[self.current.0 as usize].term = term;
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
    for f in &m.functions {
        verify_function(m, f).map_err(|e| format!("fn {}: {}", f.name, e))?;
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
    let defined = |v: NodeId| (v.0 as usize) < n;
    let jump = |(b, args): &(BlockId, Vec<NodeId>)| -> Result<(), String> {
        let target = f
            .blocks
            .get(b.0 as usize)
            .ok_or(format!("a jump to block {} that does not exist", b.0))?;
        let given: Vec<Ty> = args.iter().map(|a| f.ty(*a)).collect();
        let wanted: Vec<Ty> = target.params.iter().map(|p| f.ty(*p)).collect();
        if args.iter().any(|a| !defined(*a)) || given != wanted {
            return Err(format!("a jump to block {} with the wrong arguments", b.0));
        }
        Ok(())
    };
    for (bi, b) in f.blocks.iter().enumerate() {
        for &v in &b.insts {
            let at = || format!("v{} in block {}", v.0, bi);
            if !defined(v) || f.args(v).iter().any(|a| !defined(*a)) {
                return Err(format!("{}: an operand that does not exist", at()));
            }
            check_inst(m, f, v).map_err(|e| format!("{}: {}", at(), e))?;
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
            Term::Return(v) => {
                let given = v.filter(|v| defined(*v)).map(|v| f.ty(v));
                if given != f.result {
                    return Err(format!("block {} returns the wrong type", bi));
                }
            }
            Term::Trap(t) => {
                if f.traps.get(t.0 as usize).is_none_or(|s| s.text.len() != 1) {
                    return Err(format!("block {} traps at a site that does not exist", bi));
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
}

impl Want {
    fn admits(self, t: Ty) -> bool {
        match self {
            Want::Is(x) => x == t,
            Want::Int => matches!(t, Ty::Int(_)),
            Want::IntOrBool => matches!(t, Ty::Int(_) | Ty::Bool),
        }
    }
}

/// Checks an instruction against what its operation takes and gives: its result's type, its
/// operands' types, and the trap site it may raise, with room for the values it reports.
fn check_inst(m: &Module, f: &Function, v: NodeId) -> Result<(), String> {
    let ty = f.ty(v);
    let args: Vec<Ty> = f.args(v).iter().map(|a| f.ty(*a)).collect();
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
        Op::Slot { .. } => (Want::Is(Ty::Addr), vec![], None),
        Op::Elem { .. } => (
            Want::Is(Ty::Addr),
            vec![Want::Is(Ty::Addr), Want::Int],
            None,
        ),
        Op::Load => (Want::IntOrBool, vec![Want::Is(Ty::Addr)], None),
        Op::Store => (
            Want::Is(Ty::Unit),
            vec![Want::Is(Ty::Addr), Want::IntOrBool],
            None,
        ),
        Op::Copy { .. } => (
            Want::Is(Ty::Unit),
            vec![Want::Is(Ty::Addr), Want::Is(Ty::Addr)],
            None,
        ),
        Op::Call(c) => {
            let callee = m
                .functions
                .get(c.0 as usize)
                .ok_or("a call of a function that does not exist")?;
            let params = callee.params.iter().map(|p| Want::Is(*p)).collect();
            (Want::Is(callee.result.unwrap_or(Ty::Unit)), params, None)
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
    StackOverflow,
    /// The MIR broke one of its own rules: a lowering bug, never a program's fault.
    Broken(String),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Trap(line) => write!(f, "{}", line),
            Stop::OutOfSteps => write!(f, "the run took more steps than its limit"),
            Stop::StackOverflow => write!(f, "trap[stack-overflow]: the call stack ran out"),
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
            _ => Err(Stop::Broken(format!("a load of {}", ty))),
        }
    }

    fn copy(&mut self, dst: u64, src: u64, n: u32) -> Result<(), Stop> {
        let s = self.range(src, n as usize)?;
        let d = self.range(dst, n as usize)?;
        self.bytes.copy_within(s, d.start);
        Ok(())
    }

    fn release(&mut self, to: usize) {
        self.bytes.truncate(to);
        self.slots.retain(|(s, _)| *s < to);
    }

    /// Bytes in use: what a caller set up, and the slots of the calls still running.
    pub fn used(&self) -> usize {
        self.bytes.len()
    }
}

/// The resources a run may use.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_depth: usize,
    pub max_steps: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_depth: 10_000,
            max_steps: 100_000_000,
        }
    }
}

/// Runs function `f` of `m` on `args`, with memory a caller has set up for its addresses. Calls
/// keep their frames on a stack of the interpreter's own, so a deep recursion stops at
/// `limits.max_depth` and never at the host's stack.
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
    };
    let result = machine.run(f, args);
    (result, machine.steps)
}

struct Machine<'a> {
    m: &'a Module,
    memory: &'a mut Memory,
    limits: Limits,
    steps: u64,
}

/// A call in progress: its function, its values, where it stands, the memory it started at,
/// and the value of its caller its result goes to.
struct Frame<'m> {
    f: &'m Function,
    vals: Vec<Val>,
    block: BlockId,
    next: usize,
    mark: usize,
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
            mark: self.memory.used(),
            dest,
        };
        enter(&mut frame, BlockId(0), args)?;
        Ok(frame)
    }

    fn run(&mut self, id: FuncId, args: &[Val]) -> Result<Val, Stop> {
        let mut stack = vec![self.frame(id, args.to_vec(), None)?];
        loop {
            let depth = stack.len();
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
                    if depth >= self.limits.max_depth {
                        return Err(Stop::StackOverflow);
                    }
                    let args = f.args(v).iter().map(|a| top.vals[a.0 as usize]).collect();
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
                        _ => return Err(broken("a branch on a value that is not a bool")),
                    };
                    let args = args.iter().map(get).collect();
                    enter(top, *t, args)?;
                }
                Term::Return(v) => {
                    let result = v.as_ref().map_or(Val::Unit, get);
                    let (mark, dest) = (top.mark, top.dest);
                    stack.pop();
                    self.memory.release(mark);
                    match (stack.last_mut(), dest) {
                        (Some(caller), Some(d)) => caller.vals[d.0 as usize] = result,
                        _ => return Ok(result),
                    }
                }
                Term::Trap(t) => return Err(trap(f, *t, &[])),
            }
        }
    }

    fn inst(&mut self, f: &Function, v: NodeId, vals: &[Val]) -> Result<Val, Stop> {
        let args: Vec<Val> = f.args(v).iter().map(|a| vals[a.0 as usize]).collect();
        let ty = f.ty(v);
        let int = |i: usize| match args.get(i) {
            Some(Val::Int(n)) => Ok(*n),
            _ => Err(broken(format!("v{} wants an integer operand", v.0))),
        };
        let addr = |i: usize| match args.get(i) {
            Some(Val::Addr(a)) => Ok(*a),
            _ => Err(broken(format!("v{} wants an address operand", v.0))),
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
                _ => return Err(broken("an integer of a type that is not one")),
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
                (Some(Val::Bool(a)), _) => Val::Bool(!a),
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
                            _ => None,
                        })
                        .collect();
                    return Err(trap(f, *at, &shown));
                }
                _ => return Err(broken("a check of a value that is not a bool")),
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
                Term::Return(Some(v)) => writeln!(out, "  return v{}", v.0)?,
                Term::Return(None) => writeln!(out, "  return")?,
                Term::Trap(t) => writeln!(out, "  trap t{}", t.0)?,
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

    fn module(functions: Vec<Function>) -> Module {
        let m = Module { functions };
        verify(&m).unwrap();
        m
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
        b.end(Term::Jump(head, vec![zero, zero]));
        b.switch(head);
        let [i, s] = b.params(head)[..] else {
            panic!("two parameters")
        };
        let more = b.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n]);
        b.end(Term::Branch {
            cond: more,
            then: (body, vec![]),
            otherwise: (exit, vec![]),
        });
        b.switch(body);
        let s2 = b.inst(add(None), Ty::Int(I64), &[s, i]);
        let one = b.int(I64, Num::from_i128(1));
        let i2 = b.inst(add(None), Ty::Int(I64), &[i, one]);
        b.end(Term::Jump(head, vec![i2, s2]));
        b.switch(exit);
        b.end(Term::Return(Some(s)));
        b.finish()
    }

    #[test]
    fn test_a_function_runs_through_its_blocks() {
        let m = module(vec![sum()]);
        assert_eq!(run0(&m, 0, &[int(10)]), Ok(int(45)));
        assert_eq!(run0(&m, 0, &[int(0)]), Ok(int(0)));
        let printed = m.functions[0].to_string();
        assert!(
            printed.starts_with("fn sum(i64) -> i64 {\nblock0(v0: i64):\n  v1: i64 = int 0 @0:0\n"),
            "{}",
            printed
        );
        assert!(
            printed.contains("  branch v4, block2(), block3()\n"),
            "{}",
            printed
        );
        assert!(
            printed.contains("  v5: i64 = add v3, v2 @0:0\n"),
            "{}",
            printed
        );
    }

    #[test]
    fn test_a_module_prints_each_function_numbered_as_calls_name_it() {
        let mut b = Builder::new("twice", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let n = b.params(BlockId(0))[0];
        let s = b.inst(Op::Call(FuncId(0)), Ty::Int(I64), &[n]);
        b.end(Term::Return(Some(s)));
        let m = module(vec![sum(), b.finish()]);
        let printed = m.to_string();
        let first = m.functions[0].to_string();
        let second = m.functions[1].to_string();
        assert_eq!(printed, format!("f0: {}\nf1: {}", first, second));
        assert!(second.contains("= call f0 v0 @0:0\n"), "{}", second);
        // The same module prints the same text every time.
        assert_eq!(printed, m.to_string());
        assert_eq!(Module { functions: vec![] }.to_string(), "");
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
        b.inst(Op::Check { trap: t }, Ty::Unit, &[fits, x]);
        let y = b.inst(Op::Cast, Ty::Int(U8), &[x]);
        b.end(Term::Return(Some(y)));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(200)]), Ok(int(200)));
        assert_eq!(
            run0(&m, 0, &[int(-1)]).unwrap_err().to_string(),
            "trap[narrowing] at 3:17: -1 does not fit in `u8`"
        );
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
        r.end(Term::Return(None));
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
        t.end(Term::Trap(site));
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
            b.end(Term::Return(Some(p)));
            Module {
                functions: vec![b.finish()],
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
            b.end(Term::Return(Some(s)));
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
        b.end(Term::Return(Some(d)));
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
        b.end(Term::Return(Some(r)));
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
            let r = bl.inst(op, Ty::Int(t), &[x, y]);
            bl.end(Term::Return(Some(r)));
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
        let r = shl.inst(Op::Shl { trap: t }, Ty::Int(I64), &[x, k]);
        shl.end(Term::Return(Some(r)));
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
        let r = wide.inst(Op::Shl { trap: t }, Ty::Int(I128), &[x, k]);
        wide.end(Term::Return(Some(r)));
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
        not.end(Term::Return(Some(r)));
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
        cmp.end(Term::Return(Some(n)));
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
        let mut b = Builder::new("bump", &[Ty::Addr], None);
        let a = b.params(BlockId(0))[0];
        let one = b.int(U64, Num::from_i128(1));
        let at = b.inst(Op::Elem { stride: 1 }, Ty::Addr, &[a, one]);
        let x = b.inst(Op::Load, Ty::Int(U8), &[at]);
        let k = b.int(U8, Num::from_i128(1));
        let y = b.inst(add(None), Ty::Int(U8), &[x, k]);
        b.inst(Op::Store, Ty::Unit, &[at, y]);
        b.end(Term::Return(None));
        // `fn main() -> u8`: a slot of u16s, filled, bumped, copied, read back.
        let mut m = Builder::new("main", &[], Some(Ty::Int(U8)));
        let slot = m.inst(Op::Slot { bytes: 3 }, Ty::Addr, &[]);
        for i in 0..3 {
            let idx = m.int(U64, Num::from_i128(i));
            let at = m.inst(Op::Elem { stride: 1 }, Ty::Addr, &[slot, idx]);
            let v = m.int(U8, Num::from_i128(10 * i + 1));
            m.inst(Op::Store, Ty::Unit, &[at, v]);
        }
        m.inst(Op::Call(FuncId(0)), Ty::Unit, &[slot]);
        let copy = m.inst(Op::Slot { bytes: 3 }, Ty::Addr, &[]);
        m.inst(Op::Copy { bytes: 3 }, Ty::Unit, &[copy, slot]);
        let two = m.int(U64, Num::from_i128(2));
        let last = m.inst(Op::Elem { stride: 1 }, Ty::Addr, &[copy, two]);
        let r2 = m.inst(Op::Load, Ty::Int(U8), &[last]);
        let one = m.int(U64, Num::from_i128(1));
        let at = m.inst(Op::Elem { stride: 1 }, Ty::Addr, &[copy, one]);
        let r1 = m.inst(Op::Load, Ty::Int(U8), &[at]);
        let r = m.inst(add(None), Ty::Int(U8), &[r1, r2]);
        m.end(Term::Return(Some(r)));
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

    #[test]
    fn test_runs_stop_at_their_limits() {
        // `fn down(n: i64) -> i64 = down(n)`: recursion without end.
        let mut b = Builder::new("down", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let n = b.params(BlockId(0))[0];
        let r = b.inst(Op::Call(FuncId(0)), Ty::Int(I64), &[n]);
        b.end(Term::Return(Some(r)));
        let m = module(vec![b.finish()]);
        assert_eq!(run0(&m, 0, &[int(1)]), Err(Stop::StackOverflow));
        let m = module(vec![sum()]);
        let limits = Limits {
            max_depth: 10,
            max_steps: 100,
        };
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
        let wrong = |f: Function| verify(&Module { functions: vec![f] }).unwrap_err();
        // A jump with the wrong arguments.
        let mut f = sum();
        f.blocks[0].term = Term::Jump(BlockId(1), vec![]);
        assert!(wrong(f).contains("wrong arguments"));
        let mut f = sum();
        f.blocks[0].term = Term::Jump(BlockId(9), vec![]);
        assert!(wrong(f).contains("does not exist"));
        // A return of the wrong type.
        let mut f = sum();
        f.blocks[3].term = Term::Return(None);
        assert!(wrong(f).contains("returns the wrong type"));
        // An addition of an i64 and a bool.
        let mut b = Builder::new("bad", &[Ty::Int(I64)], None);
        let x = b.params(BlockId(0))[0];
        let t = b.inst(Op::Bool(true), Ty::Bool, &[]);
        b.inst(add(None), Ty::Int(I64), &[x, t]);
        assert!(wrong(b.finish()).contains("Arith"));
        // A call of a function that does not exist, and a trap site that does not.
        let mut b = Builder::new("call", &[], None);
        b.inst(Op::Call(FuncId(7)), Ty::Unit, &[]);
        assert!(wrong(b.finish()).contains("does not exist"));
        let mut b = Builder::new("trap", &[], None);
        b.end(Term::Trap(TrapId(0)));
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
    /// parameters of `params`; `traps` are the sites' texts.
    fn one(op: Op, ty: Ty, params: &[Ty], args: &[usize], traps: &[&[&str]]) -> Function {
        let mut b = Builder::new("one", params, None);
        let ps = b.params(BlockId(0));
        for t in traps {
            b.trap_site("narrowing", t);
        }
        let operands: Vec<NodeId> = args.iter().map(|i| ps[*i]).collect();
        b.inst(op, ty, &operands);
        b.finish()
    }

    #[test]
    fn test_the_verifier_checks_each_instruction_against_its_operation() {
        let check = |f: Function| verify(&Module { functions: vec![f] });
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
            one(
                Op::Elem { stride: 1 },
                Ty::Addr,
                &[Ty::Addr, u],
                &[0, 1],
                &[],
            ),
            one(Op::Store, Ty::Unit, &[Ty::Addr, Ty::Bool], &[0, 1], &[]),
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
                Ty::Addr,
                &[Ty::Addr, Ty::Bool],
                &[0, 1],
                &[],
            ),
            one(Op::Load, Ty::Addr, &[Ty::Addr], &[0], &[]),
            one(Op::Store, Ty::Unit, &[i, i], &[0, 1], &[]),
            one(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, u], &[0, 1], &[]),
            one(
                Op::Cmp(CmpOp::Lt),
                Ty::Bool,
                &[Ty::Addr, Ty::Addr],
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
            one(Op::Not, Ty::Addr, &[Ty::Addr], &[0], &[]),
            one(Op::Bits(BitOp::And), u, &[u, i], &[0, 1], &[]),
            one(
                Op::Copy { bytes: 1 },
                Ty::Unit,
                &[Ty::Addr, u],
                &[0, 1],
                &[],
            ),
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
        callee.end(Term::Return(Some(w)));
        let callee = callee.finish();
        let caller = |ty: Ty, param: Ty| {
            let mut b = Builder::new("caller", &[param], None);
            let p = b.params(BlockId(0))[0];
            b.inst(Op::Call(FuncId(0)), ty, &[p]);
            Module {
                functions: vec![callee.clone(), b.finish()],
            }
        };
        assert_eq!(verify(&caller(i, u)), Ok(()));
        assert!(verify(&caller(u, u)).is_err());
        assert!(verify(&caller(i, i)).is_err());
        // An operand one past the last value.
        let mut f = sum();
        let last = NodeId(f.ops.len() as u32);
        f.pool[0] = last;
        assert!(verify(&Module { functions: vec![f] })
            .unwrap_err()
            .contains("does not exist"));
    }

    /// A slot read before it is written holds the memory's fill, 0 unless another is asked
    /// for, and a copy of memory starts runs of its own; a run counts what it executed.
    #[test]
    fn test_unwritten_memory_holds_its_fill_and_runs_count_their_steps() {
        // `fn peek() -> u16`: a slot's first two bytes, never written.
        let mut b = Builder::new("peek", &[], Some(Ty::Int(IntTy::new(16, false))));
        let s = b.inst(Op::Slot { bytes: 2 }, Ty::Addr, &[]);
        let v = b.inst(Op::Load, Ty::Int(IntTy::new(16, false)), &[s]);
        b.end(Term::Return(Some(v)));
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
            let s = b.inst(Op::Slot { bytes: 3 }, Ty::Addr, &[]);
            let _next = b.inst(Op::Slot { bytes: 4 }, Ty::Addr, &[]);
            let e = b.inst(Op::Elem { stride: 1 }, Ty::Addr, &[s, i]);
            let v = b.inst(Op::Load, Ty::Int(u16), &[e]);
            b.end(Term::Return(Some(v)));
            run(
                &module(vec![b.finish()]),
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
        memory.alloc(2);
        memory.release(16);
        assert_eq!(memory.load(kept, Ty::Int(U8)), Ok(int(0)));
        assert!(memory.load(16, Ty::Int(U8)).is_err());
    }

    #[test]
    fn test_runs_count_steps_and_give_back_memory() {
        // Three instructions: a slot, a constant and its store.
        let mut b = Builder::new("three", &[], None);
        let s = b.inst(Op::Slot { bytes: 8 }, Ty::Addr, &[]);
        let k = b.int(I64, Num::from_i128(7));
        b.inst(Op::Store, Ty::Unit, &[s, k]);
        b.end(Term::Return(None));
        let m = module(vec![b.finish()]);
        let limited = |steps| Limits {
            max_depth: 10,
            max_steps: steps,
        };
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
            let r = b.inst(Op::Shr, Ty::Int(I128), &[x, k]);
            b.end(Term::Return(Some(r)));
            run0(&module(vec![b.finish()]), 0, &[int(8), int(amount)])
        };
        assert_eq!(shr(127), Ok(int(0)));
        assert!(matches!(shr(-1), Err(Stop::Broken(_))));
        assert!(matches!(shr(128), Err(Stop::Broken(_))));
        // A negative constant prints as its value.
        let mut b = Builder::new("k", &[], Some(Ty::Int(I64)));
        let k = b.int(I64, Num::from_i128(-5));
        b.end(Term::Return(Some(k)));
        assert!(b.finish().to_string().contains("= int -5 @"));
    }
}

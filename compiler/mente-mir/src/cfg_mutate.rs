//! Mutation operators over the SSA MIR of [`crate::cfg`] (`spec/mir-and-codegen.md` item 4).
//!
//! A mutant is one small change to one instruction or terminator of a lowered function, made
//! in memory on a copy of the module. A test set that tells every mutant apart from the
//! original, or that is shown to be unable to only because the mutant computes the same thing,
//! checks what the code does rather than that it runs. Each operator family covers a kind of
//! instruction or terminator the lowering emits:
//!
//! - [`Family::Arith`]: `+` and `-` swapped, `*` to `/`, `/` and `%` swapped;
//! - [`Family::Bits`]: each of `and`, `or` and `xor` to each of the other two, on integers and on
//!   booleans;
//! - [`Family::Cmp`]: each comparison to its negation and to its neighbour, `<` and `<=`, `>`
//!   and `>=`, `==` and `!=`;
//! - [`Family::Const`]: an integer constant `c` to `c + 1`, `c - 1` and `0` where its type holds
//!   them, a boolean constant flipped;
//! - [`Family::Branch`]: a conditional branch's two targets swapped, and the branch replaced by
//!   a jump to either target, which also replaces an `if` value by either arm, since MIR has no
//!   select and the lowering joins an `if` value in a block parameter;
//! - [`Family::Trap`]: a run-time check taken out, so that its trap never fires, and a checked
//!   operation made exact, so that where it would trap the MIR interpreter reports broken MIR: a
//!   count of a call's frame, `enter`, included, which then never runs out of the budget;
//! - [`Family::Identity`]: a negation, a `not` or a shift replaced by its first operand;
//! - [`Family::Memory`]: a store or a copy taken out.
//!
//! Not mutated: a cast, which only moves a value between types that hold it; a slot, an element
//! address or a load, whose change reads other bytes of the same frame rather than another value
//! the program computes; a call; a count moved, dropped or doubled, every one of which the
//! verifier refuses, as [`unpaired`] shows; the count in an entry, of the run's first call, which
//! never traps, since a call from outside whose frame is larger than the budget is refused before
//! the entry runs; the chain of effects broken, which the verifier refuses, as [`unchained`]
//! shows; and the terminators `jump`, `return` and `trap`, whose targets
//! and values the other families already reach through the instructions that compute them.
//!
//! A mutant is named by its function, its block, the instruction's index in that block or
//! `term`, and its operator: `clamp/b2.i3/lt>le`. The name stays the same through any change to
//! another function and to the instructions after it in its block, so a record that a mutant
//! is equivalent survives unrelated edits as far as an index can.

use crate::cfg::{self, ArithOp, BitOp, BlockId, Chain, CmpOp, FuncId, Module, Num, Op, Term, Ty};
use crate::NodeId;

/// A group of operators, by the kind of instruction or terminator they change.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Family {
    Arith,
    Bits,
    Cmp,
    Const,
    Branch,
    Trap,
    Identity,
    Memory,
}

impl Family {
    pub const ALL: [Family; 8] = [
        Family::Arith,
        Family::Bits,
        Family::Cmp,
        Family::Const,
        Family::Branch,
        Family::Trap,
        Family::Identity,
        Family::Memory,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Family::Arith => "arith",
            Family::Bits => "bits",
            Family::Cmp => "cmp",
            Family::Const => "const",
            Family::Branch => "branch",
            Family::Trap => "trap",
            Family::Identity => "identity",
            Family::Memory => "memory",
        }
    }
}

/// Where in a block a mutant changes something.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Site {
    /// The instruction at this index of the block's instructions.
    Inst(u32),
    /// The block's terminator.
    Term,
}

/// What a mutant changes at its site.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    /// The instruction's operation becomes this one; its operands and type stay.
    Op(Op),
    /// The instruction leaves its block: a check that never fires, a store or copy never made.
    Remove,
    /// Every use of the instruction's result reads its first operand instead.
    Bypass,
    /// The branch goes to its second target when the condition holds, and to its first when not.
    SwapTargets,
    /// The branch becomes a jump to its first target (`true`) or its second (`false`).
    Always(bool),
}

/// One mutant of a module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mutant {
    pub func: FuncId,
    pub block: BlockId,
    pub site: Site,
    pub family: Family,
    /// The operator as the name shows it: `add>sub`, `int+1`, `check-removed`, `swap`.
    pub operator: &'static str,
    pub change: Change,
}

impl Mutant {
    /// The mutant's stable name, given what its function is called: `clamp/b2.i3/lt>le`.
    pub fn name(&self, func: &str) -> String {
        let at = match self.site {
            Site::Inst(i) => format!("i{}", i),
            Site::Term => "term".to_string(),
        };
        format!("{}/b{}.{}/{}", func, self.block.0, at, self.operator)
    }

    /// The value the mutant changes, when its site is an instruction.
    pub fn value(&self, m: &Module) -> Option<NodeId> {
        let f = m.functions.get(self.func.0 as usize)?;
        match self.site {
            Site::Inst(i) => f
                .blocks
                .get(self.block.0 as usize)?
                .insts
                .get(i as usize)
                .copied(),
            Site::Term => None,
        }
    }

    /// The source line and column of what the mutant changes: its instruction's, or for a
    /// terminator its condition's.
    pub fn loc(&self, m: &Module) -> cfg::Loc {
        let Some(f) = m.functions.get(self.func.0 as usize) else {
            return cfg::Loc::default();
        };
        let at = match self.site {
            Site::Inst(_) => self.value(m),
            Site::Term => match f.blocks.get(self.block.0 as usize).map(|b| &b.term) {
                Some(Term::Branch { cond, .. }) => Some(*cond),
                _ => None,
            },
        };
        at.and_then(|v| f.locs.get(v.0 as usize).copied())
            .unwrap_or_default()
    }

    /// A copy of `m` with this mutant's change made; `m` itself is left alone.
    pub fn apply(&self, m: &Module) -> Module {
        let mut out = m.clone();
        let Some(f) = out.functions.get_mut(self.func.0 as usize) else {
            return out;
        };
        let Some(block) = f.blocks.get_mut(self.block.0 as usize) else {
            return out;
        };
        match (self.site, &self.change) {
            (Site::Inst(i), change) => {
                let Some(&v) = block.insts.get(i as usize) else {
                    return out;
                };
                match change {
                    Change::Op(op) => {
                        let was = f.op(v).chain();
                        f.ops[v.0 as usize] = op.clone();
                        // An effect made a partial operation, as a checked one made exact is,
                        // leaves the chain: what took it takes the effect it took.
                        match (was, op.chain()) {
                            (Chain::Follows, Chain::Reads) => {
                                if let Some(before) = f.args(v).last().copied() {
                                    replace_effect_uses(f, v, before);
                                }
                            }
                            // One that takes no effect, as an exact division, lets go of it too.
                            (Chain::Follows, Chain::Free) => {
                                if let Some(before) = f.args(v).last().copied() {
                                    replace_effect_uses(f, v, before);
                                    let (start, len) = f.operands[v.0 as usize];
                                    f.operands[v.0 as usize] = (start, len.saturating_sub(1));
                                }
                            }
                            (Chain::Follows, Chain::Follows)
                            | (
                                Chain::Reads | Chain::Free,
                                Chain::Follows | Chain::Reads | Chain::Free,
                            ) => {}
                        }
                    }
                    Change::Remove => match f.op(v).fact() {
                        // A guard that never fires: its value flows on, assumed.
                        Some(fact) => {
                            let args = f.args(v).to_vec();
                            if let [.., guarded, effect] = args[..] {
                                let start = f.pool.len() as u32;
                                f.pool.extend([guarded, effect]);
                                f.operands[v.0 as usize] = (start, 2);
                                f.ops[v.0 as usize] = Op::Assume(fact);
                            }
                        }
                        None => {
                            f.blocks[self.block.0 as usize].insts.remove(i as usize);
                            // What took the effect taken out takes the one it took.
                            if let Some(before) = f.effect(v) {
                                replace_effect_uses(f, v, before);
                            }
                        }
                    },
                    Change::Bypass => {
                        let Some(&first) = f.args(v).first() else {
                            return out;
                        };
                        replace_value_uses(f, v, first);
                    }
                    Change::SwapTargets | Change::Always(_) => {}
                }
            }
            (Site::Term, change) => {
                let Term::Branch {
                    cond,
                    then,
                    otherwise,
                } = &block.term
                else {
                    return out;
                };
                block.term = match change {
                    Change::SwapTargets => Term::Branch {
                        cond: *cond,
                        then: otherwise.clone(),
                        otherwise: then.clone(),
                    },
                    Change::Always(true) => Term::Jump(then.0, then.1.clone()),
                    Change::Always(false) => Term::Jump(otherwise.0, otherwise.1.clone()),
                    Change::Op(_) | Change::Remove | Change::Bypass => return out,
                };
            }
        }
        out
    }
}

/// Where each operand and terminator of `f` takes an effect, which the chain of effects holds
/// ([`Chain`]), and where it takes a value.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Taken {
    Effect,
    Value,
}

/// Every use of `old` in `f` where it is taken as `taken` says, as an operand or in a
/// terminator, reads `new` instead.
fn replace(f: &mut cfg::Function, old: NodeId, new: NodeId, taken: Taken) {
    let swap = |v: &mut NodeId| {
        if *v == old {
            *v = new;
        }
    };
    for i in 0..f.ops.len() {
        let (start, len) = f.operands[i];
        let (start, len) = (start as usize, len as usize);
        let effect = match f.ops[i].chain() {
            Chain::Follows | Chain::Reads => len.checked_sub(1),
            Chain::Free => None,
        };
        for k in 0..len {
            let here = if Some(k) == effect {
                Taken::Effect
            } else {
                Taken::Value
            };
            if here == taken {
                swap(&mut f.pool[start + k]);
            }
        }
    }
    // A terminator takes the latest effect last: a jump's and a branch's last arguments, a
    // return's and a trap's effect.
    let args = |args: &mut Vec<NodeId>| {
        let last = args.len().saturating_sub(1);
        for (k, a) in args.iter_mut().enumerate() {
            let here = if k == last {
                Taken::Effect
            } else {
                Taken::Value
            };
            if here == taken {
                swap(a);
            }
        }
    };
    for b in &mut f.blocks {
        match &mut b.term {
            Term::Jump(_, a) => args(a),
            Term::Branch {
                cond,
                then,
                otherwise,
            } => {
                if taken == Taken::Value {
                    swap(cond);
                }
                args(&mut then.1);
                args(&mut otherwise.1);
            }
            Term::Return(v, effect) => match taken {
                Taken::Value => v.iter_mut().for_each(swap),
                Taken::Effect => swap(effect),
            },
            Term::Trap(_, effect) => {
                if taken == Taken::Effect {
                    swap(effect);
                }
            }
        }
    }
}

/// Every use of `old` as an effect reads `new` instead.
fn replace_effect_uses(f: &mut cfg::Function, old: NodeId, new: NodeId) {
    replace(f, old, new, Taken::Effect);
}

/// Every use of `old` as a value reads `new` instead; what takes it as an effect still does.
fn replace_value_uses(f: &mut cfg::Function, old: NodeId, new: NodeId) {
    replace(f, old, new, Taken::Value);
}

/// Every mutant the operators make of `m`, function by function, block by block, in order. Some
/// may break MIR's rules, as replacing a shift by an operand of another type does; [`split`]
/// sets those apart.
pub fn mutants(m: &Module) -> Vec<Mutant> {
    let mut out = Vec::new();
    for (fi, f) in m.functions.iter().enumerate() {
        let func = FuncId(fi as u32);
        for (bi, b) in f.blocks.iter().enumerate() {
            let block = BlockId(bi as u32);
            for (ii, &v) in b.insts.iter().enumerate() {
                let site = Site::Inst(ii as u32);
                for (family, operator, change) in inst_changes(f, v) {
                    out.push(Mutant {
                        func,
                        block,
                        site,
                        family,
                        operator,
                        change,
                    });
                }
            }
            if let Term::Branch { .. } = b.term {
                for (operator, change) in [
                    ("swap", Change::SwapTargets),
                    ("then", Change::Always(true)),
                    ("else", Change::Always(false)),
                ] {
                    out.push(Mutant {
                        func,
                        block,
                        site: Site::Term,
                        family: Family::Branch,
                        operator,
                        change,
                    });
                }
            }
        }
    }
    out
}

/// The mutants of `m` that pass [`cfg::verify`], and those that do not, with why.
pub fn split(m: &Module) -> (Vec<Mutant>, Vec<(Mutant, String)>) {
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for mutant in mutants(m) {
        match cfg::verify(&mutant.apply(m)) {
            Ok(()) => kept.push(mutant),
            Err(why) => skipped.push((mutant, why)),
        }
    }
    (kept, skipped)
}

type Candidate = (Family, &'static str, Change);

/// The changes the operators make to the instruction `v` of `f`.
fn inst_changes(f: &cfg::Function, v: NodeId) -> Vec<Candidate> {
    let op = |o: Op| Change::Op(o);
    match f.op(v) {
        Op::Arith { op: a, trap } => {
            let to = |b: ArithOp| op(Op::Arith { op: b, trap: *trap });
            let mut out: Vec<Candidate> = match a {
                ArithOp::Add => vec![(Family::Arith, "add>sub", to(ArithOp::Sub))],
                ArithOp::Sub => vec![(Family::Arith, "sub>add", to(ArithOp::Add))],
                ArithOp::Mul => vec![(Family::Arith, "mul>div", to(ArithOp::Div))],
                ArithOp::Div => vec![(Family::Arith, "div>rem", to(ArithOp::Rem))],
                ArithOp::Rem => vec![(Family::Arith, "rem>div", to(ArithOp::Div))],
            };
            if trap.is_some() {
                out.push((
                    Family::Trap,
                    "unchecked",
                    op(Op::Arith { op: *a, trap: None }),
                ));
            }
            out
        }
        Op::Neg { trap } => {
            let mut out = vec![(Family::Identity, "neg>id", Change::Bypass)];
            if trap.is_some() {
                out.push((Family::Trap, "unchecked", op(Op::Neg { trap: None })));
            }
            out
        }
        Op::Bits(b) => {
            let to = |c: BitOp| op(Op::Bits(c));
            match b {
                BitOp::And => vec![
                    (Family::Bits, "and>or", to(BitOp::Or)),
                    (Family::Bits, "and>xor", to(BitOp::Xor)),
                ],
                BitOp::Or => vec![
                    (Family::Bits, "or>and", to(BitOp::And)),
                    (Family::Bits, "or>xor", to(BitOp::Xor)),
                ],
                BitOp::Xor => vec![
                    (Family::Bits, "xor>and", to(BitOp::And)),
                    (Family::Bits, "xor>or", to(BitOp::Or)),
                ],
            }
        }
        Op::Not => vec![(Family::Identity, "not>id", Change::Bypass)],
        Op::Shl { .. } => vec![(Family::Identity, "shl>id", Change::Bypass)],
        Op::Shr => vec![(Family::Identity, "shr>id", Change::Bypass)],
        Op::Cmp(c) => {
            let to = |d: CmpOp| op(Op::Cmp(d));
            match c {
                CmpOp::Lt => vec![
                    (Family::Cmp, "lt>ge", to(CmpOp::Ge)),
                    (Family::Cmp, "lt>le", to(CmpOp::Le)),
                ],
                CmpOp::Le => vec![
                    (Family::Cmp, "le>gt", to(CmpOp::Gt)),
                    (Family::Cmp, "le>lt", to(CmpOp::Lt)),
                ],
                CmpOp::Gt => vec![
                    (Family::Cmp, "gt>le", to(CmpOp::Le)),
                    (Family::Cmp, "gt>ge", to(CmpOp::Ge)),
                ],
                CmpOp::Ge => vec![
                    (Family::Cmp, "ge>lt", to(CmpOp::Lt)),
                    (Family::Cmp, "ge>gt", to(CmpOp::Gt)),
                ],
                CmpOp::Eq => vec![(Family::Cmp, "eq>ne", to(CmpOp::Ne))],
                CmpOp::Ne => vec![(Family::Cmp, "ne>eq", to(CmpOp::Eq))],
            }
        }
        Op::Int(bits) => {
            let Ty::Int(t) = f.ty(v) else {
                return Vec::new();
            };
            let n = Num::from_bits(*bits, t);
            [
                ("int+1", step(n, true)),
                ("int-1", step(n, false)),
                ("int>0", Some(Num::ZERO)),
            ]
            .into_iter()
            .fold(Vec::new(), |mut out: Vec<(Num, Candidate)>, (name, to)| {
                // `-1 + 1` is already `0`: one mutant for one new value.
                if let Some(to) = to.filter(|c| *c != n && t.holds(*c)) {
                    if out.iter().all(|(seen, _)| *seen != to) {
                        out.push((to, (Family::Const, name, op(Op::Int(to.to_bits(t))))));
                    }
                }
                out
            })
            .into_iter()
            .map(|(_, c)| c)
            .collect()
        }
        Op::Bool(b) => vec![(Family::Const, "flip", op(Op::Bool(!b)))],
        Op::Check { .. } | Op::Guard { .. } => {
            vec![(Family::Trap, "check-removed", Change::Remove)]
        }
        Op::Store => vec![(Family::Memory, "store-removed", Change::Remove)],
        Op::Copy { .. } => vec![(Family::Memory, "copy-removed", Change::Remove)],
        // A count made exact never traps where the stack runs out. An entry, of a frame of 0,
        // counts the run's first call, which a frame larger than the budget never reaches.
        Op::Enter {
            callee,
            trap: Some(_),
        } if f.frame > 0 => vec![(
            Family::Trap,
            "enter-exact",
            op(Op::Enter {
                callee: *callee,
                trap: None,
            }),
        )],
        Op::Param
        | Op::Cast
        | Op::Assume(_)
        | Op::Slot { .. }
        | Op::Elem { .. }
        | Op::Load
        | Op::Call(_)
        | Op::Enter { .. } => Vec::new(),
    }
}

/// The callee whose frame the instruction `v` of `f` counts, where it is an [`Op::Enter`].
fn counted(f: &cfg::Function, v: NodeId) -> Option<FuncId> {
    match f.op(v) {
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

/// The callee of the instruction `v` of `f`, where it is an [`Op::Call`].
fn called(f: &cfg::Function, v: NodeId) -> Option<FuncId> {
    match f.op(v) {
        Op::Call(c) => Some(*c),
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
        | Op::Enter { .. } => None,
    }
}

/// Where the call of `callee` that an `Enter` at `block`, `at` opens stands: the first call of
/// `callee` after it in its block, or else in a block after its own.
fn its_call(f: &cfg::Function, block: usize, at: usize, callee: FuncId) -> Option<(usize, usize)> {
    let is_call = |v: NodeId| called(f, v) == Some(callee);
    let later = f.blocks[block].insts[at + 1..]
        .iter()
        .position(|&v| is_call(v))
        .map(|i| (block, at + 1 + i));
    later.or_else(|| {
        f.blocks
            .iter()
            .enumerate()
            .skip(block + 1)
            .find_map(|(bi, b)| b.insts.iter().position(|&v| is_call(v)).map(|i| (bi, i)))
    })
}

/// Each way a change could break the count of a call's frame (`core-calculus.md`, item 25): an
/// [`Op::Enter`] of `m` dropped, doubled, moved past its call, moved past the first narrowing or
/// check that takes it, or moved before an argument it takes, one module for each `Enter` and
/// each way, named as a mutant is. The verifier refuses every one ([`cfg::verify`]): a count
/// taken where it is not defined, or a call that does not close the latest count open. They are
/// not mutants of the gate, whose mutants verify: they show that the count's place is held by
/// MIR's values.
pub fn unpaired(m: &Module) -> Vec<(String, Module)> {
    let mut out = Vec::new();
    for (fi, f) in m.functions.iter().enumerate() {
        for (bi, b) in f.blocks.iter().enumerate() {
            for (ii, &v) in b.insts.iter().enumerate() {
                let Some(callee) = counted(f, v) else {
                    continue;
                };
                let name = |what: &str| format!("{}/b{}.i{}/{}", f.name, bi, ii, what);
                let mut dropped = m.clone();
                dropped.functions[fi].blocks[bi].insts.remove(ii);
                out.push((name("enter-dropped"), dropped));
                let mut doubled = m.clone();
                let g = &mut doubled.functions[fi];
                let twin = NodeId(g.ops.len() as u32);
                g.ops.push(g.ops[v.0 as usize].clone());
                g.tys.push(g.tys[v.0 as usize]);
                g.locs.push(g.locs[v.0 as usize]);
                let start = g.pool.len() as u32;
                let operands = f.args(v).to_vec();
                g.pool.extend(&operands);
                g.operands.push((start, operands.len() as u32));
                g.blocks[bi].insts.insert(ii + 1, twin);
                out.push((name("enter-doubled"), doubled));
                let moved_to = |to: usize| {
                    let mut moved = m.clone();
                    let insts = &mut moved.functions[fi].blocks[bi].insts;
                    insts.remove(ii);
                    insts.insert(to, v);
                    moved
                };
                if let Some((cb, ci)) = its_call(f, bi, ii, callee) {
                    let mut moved = m.clone();
                    let g = &mut moved.functions[fi];
                    g.blocks[cb].insts.insert(ci + 1, v);
                    g.blocks[bi].insts.remove(ii);
                    out.push((name("enter-moved"), moved));
                }
                // Past the first instruction after it, other than its call, that takes it.
                let takes = b.insts[ii + 1..]
                    .iter()
                    .position(|&u| f.effect(u) == Some(v) && called(f, u).is_none());
                if let Some(k) = takes {
                    out.push((name("enter-later"), moved_to(ii + 1 + k)));
                }
                // Before the last of its arguments that an instruction of its block gives.
                let given = b.insts[..ii]
                    .iter()
                    .rposition(|u| f.own_args(v).contains(u));
                if let Some(k) = given {
                    out.push((name("enter-earlier"), moved_to(k)));
                }
            }
            // Each call past the next call of its block.
            for (ii, &v) in b.insts.iter().enumerate() {
                if called(f, v).is_none() {
                    continue;
                }
                let next = b.insts[ii + 1..]
                    .iter()
                    .position(|&u| called(f, u).is_some());
                if let Some(k) = next {
                    let mut moved = m.clone();
                    let insts = &mut moved.functions[fi].blocks[bi].insts;
                    insts.remove(ii);
                    insts.insert(ii + 1 + k, v);
                    out.push((format!("{}/b{}.i{}/call-later", f.name, bi, ii), moved));
                }
            }
        }
    }
    out
}

/// Each way a change could break the chain of effects ([`cfg::Chain`]), one module for each site
/// and way, named as a mutant is:
///
/// - `effect-dropped`: an effect or a partial operation without the effect it takes;
/// - `effect-forked`: an effect taking the effect its own effect took, which two then take;
/// - `effect-valued`: an effect or a partial operation taking a value that is no effect, its
///   first operand, in place of the effect;
/// - `partial-raised`: an exact operation moved above the effect it reads, in its block;
/// - `guard-unguarded`: a partial operation, or the cast between it and its guard, taking the
///   value its guard takes in place of the one the guard yields;
/// - `guard-raised`: that, and the partial operation moved above its guard, as a pass that hoists
///   it past the guard would leave it;
/// - `passed-valued`: a jump or a branch passing a value that is no effect, the branch's condition
///   or the jump's first argument, to its target's effect parameter;
/// - `ended-early`: a return or a trap taking its block's effect parameter where an effect of the
///   block came since, which nothing would then take.
///
/// The verifier refuses every one ([`cfg::verify`]). They are not mutants of the gate: they show
/// that the chain is held whole, by the verifier, whatever a change moves or rewrites.
pub fn unchained(m: &Module) -> Vec<(String, Module)> {
    let mut out = Vec::new();
    for (fi, f) in m.functions.iter().enumerate() {
        let changed = |name: String, change: &dyn Fn(&mut cfg::Function)| {
            let mut copy = m.clone();
            change(&mut copy.functions[fi]);
            (name, copy)
        };
        for (bi, b) in f.blocks.iter().enumerate() {
            for (ii, &v) in b.insts.iter().enumerate() {
                let name = |what: &str| format!("{}/b{}.i{}/{}", f.name, bi, ii, what);
                let role = f.op(v).chain();
                let Some(effect) = f.effect(v) else {
                    continue;
                };
                out.push(changed(name("effect-dropped"), &|g| {
                    let (start, len) = g.operands[v.0 as usize];
                    g.operands[v.0 as usize] = (start, len.saturating_sub(1));
                }));
                let last = |g: &mut cfg::Function, to: NodeId| {
                    let (start, len) = g.operands[v.0 as usize];
                    if let Some(k) = (start + len).checked_sub(1) {
                        g.pool[k as usize] = to;
                    }
                };
                let forked = match role {
                    Chain::Follows => f.effect(effect),
                    Chain::Reads | Chain::Free => None,
                };
                if let Some(earlier) = forked {
                    out.push(changed(name("effect-forked"), &|g| last(g, earlier)));
                }
                // Its first operand, where that is not also the effect, as a checked square's is.
                if let Some(&value) = f.own_args(v).first().filter(|a| **a != effect) {
                    out.push(changed(name("effect-valued"), &|g| last(g, value)));
                }
                // A partial operation above the effect it reads, where that effect is an
                // instruction of its block.
                let raised = match role {
                    Chain::Reads => b.insts.iter().position(|u| *u == effect),
                    Chain::Follows | Chain::Free => None,
                };
                if let Some(at) = raised {
                    out.push(changed(name("partial-raised"), &|g| {
                        let insts = &mut g.blocks[bi].insts;
                        insts.retain(|u| *u != v);
                        insts.insert(at, v);
                    }));
                }
            }
            for (ii, &v) in b.insts.iter().enumerate() {
                let name = |what: &str| format!("{}/b{}.i{}/{}", f.name, bi, ii, what);
                let Some((k, need)) = f.partial_in(v) else {
                    continue;
                };
                // The guard beneath the operand, through the casts that keep its value, and the
                // instruction that takes it, with the place it takes it at.
                let (mut taker, mut at) = (v, k);
                let mut guard = f.own_args(v).get(k).copied();
                while let Some(c) = guard.filter(|c| *f.op(*c) == Op::Cast) {
                    (taker, at) = (c, 0);
                    guard = f.own_args(c).first().copied();
                }
                // Where what the guard takes holds the need by itself, as a constant can, the
                // guard rules nothing out, and nothing is planted.
                let Some((guard, raw)) = guard
                    .filter(|g| f.op(*g).fact().is_some())
                    .and_then(|g| f.own_args(g).last().map(|raw| (g, *raw)))
                    .filter(|(_, raw)| !f.fact(*raw).is_some_and(|fact| fact.implies(need.fact())))
                else {
                    continue;
                };
                let point = move |g: &mut cfg::Function| {
                    let (start, _) = g.operands[taker.0 as usize];
                    g.pool[start as usize + at] = raw;
                };
                out.push(changed(name("guard-unguarded"), &|g| point(g)));
                let above = b.insts.iter().position(|u| *u == guard);
                if let (true, Some(above)) = (taker == v, above) {
                    out.push(changed(name("guard-raised"), &|g| {
                        point(g);
                        let insts = &mut g.blocks[bi].insts;
                        insts.retain(|u| *u != v);
                        insts.insert(above, v);
                    }));
                }
            }
            let name = |what: &str| format!("{}/b{}.term/{}", f.name, bi, what);
            // The latest effect where the block ends: its last effect, or its effect parameter.
            let latest = b
                .insts
                .iter()
                .rev()
                .find(|u| f.op(**u).chain() == Chain::Follows)
                .or(b.params.last())
                .copied();
            let passed = match &b.term {
                Term::Jump(_, args) => args.first().copied(),
                Term::Branch { cond, .. } => Some(*cond),
                Term::Return(..) | Term::Trap(..) => None,
            }
            .filter(|value| Some(*value) != latest);
            if let Some(value) = passed {
                out.push(changed(name("passed-valued"), &|g| {
                    set_effect(&mut g.blocks[bi].term, value)
                }));
            }
            // A return or a trap after an effect of its block, taking the block's entry effect.
            let since = b.insts.iter().any(|u| f.op(*u).chain() == Chain::Follows);
            let ended = match &b.term {
                Term::Return(..) | Term::Trap(..) => b.params.last().copied().filter(|_| since),
                Term::Jump(..) | Term::Branch { .. } => None,
            };
            if let Some(entry) = ended {
                out.push(changed(name("ended-early"), &|g| {
                    set_effect(&mut g.blocks[bi].term, entry)
                }));
            }
        }
    }
    out
}

/// The effect a terminator takes made `to`: a jump's, a branch's first target's, a return's or a
/// trap's.
fn set_effect(term: &mut Term, to: NodeId) {
    match term {
        Term::Jump(_, args)
        | Term::Branch {
            then: (_, args), ..
        } => {
            if let Some(e) = args.last_mut() {
                *e = to;
            }
        }
        Term::Return(_, e) | Term::Trap(_, e) => *e = to,
    }
}

/// `n + 1` when `up`, else `n - 1`, when its magnitude stays below 2^128.
fn step(n: Num, up: bool) -> Option<Num> {
    Some(match (n.neg, up) {
        (false, true) | (true, false) => Num::new(n.neg, n.mag.checked_add(1)?),
        (_, _) if n.mag == 0 => Num::new(true, 1),
        (_, _) => Num::new(n.neg, n.mag - 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::{Builder, Fact, IntTy, Limits, Memory, Stop, Val};

    const I64: IntTy = IntTy::new(64, true);
    const U8: IntTy = IntTy::new(8, false);

    fn run(m: &Module, args: &[Val]) -> Result<Val, Stop> {
        cfg::run(
            m,
            FuncId(0),
            args,
            &mut Memory::default(),
            Limits { max_steps: 10_000 },
        )
    }

    fn int(n: i128) -> Val {
        Val::Int(Num::from_i128(n))
    }

    /// `fn sum(n: i64) -> i64`: 0 + 1 + ... + (n - 1), through a loop of blocks.
    fn sum() -> Module {
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
        let add = Op::Arith {
            op: ArithOp::Add,
            trap: None,
        };
        let s2 = b.inst(add.clone(), Ty::Int(I64), &[s, i]);
        let one = b.int(I64, Num::from_i128(1));
        let i2 = b.inst(add, Ty::Int(I64), &[i, one]);
        b.jump(head, vec![i2, s2]);
        b.switch(exit);
        b.ret(Some(s));
        let m = Module {
            functions: vec![b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        cfg::verify(&m).unwrap();
        m
    }

    /// `fn callee(x: u8) -> u8 = x`, of a frame of 100 bytes, `fn id(x: i64) -> i64 = x`, and
    /// `fn caller(x: i64) -> u8 = callee(id(x))`, which counts and calls `id`, then counts
    /// `callee` on what `id` gave, narrows that with a check, and calls `callee`, as the lowering
    /// does.
    fn counted_call() -> Module {
        let mut c = Builder::new("callee", &[Ty::Int(U8)], Some(Ty::Int(U8)));
        let x = c.params(BlockId(0))[0];
        c.ret(Some(x));
        let mut callee = c.finish();
        callee.frame = 100;
        let mut i = Builder::new("id", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = i.params(BlockId(0))[0];
        i.ret(Some(x));
        let mut id = i.finish();
        id.frame = 72;
        let mut b = Builder::new("caller", &[Ty::Int(I64)], Some(Ty::Int(U8)));
        let x = b.params(BlockId(0))[0];
        let deep =
            |b: &mut Builder| Some(b.trap_site("stack-overflow", &["the call stack ran out"]));
        let trap = deep(&mut b);
        let inner = b.inst(
            Op::Enter {
                callee: FuncId(1),
                trap,
            },
            Ty::Unit,
            &[x],
        );
        let _ = inner;
        let y = b.inst(Op::Call(FuncId(1)), Ty::Int(I64), &[x]);
        let trap = deep(&mut b);
        b.inst(
            Op::Enter {
                callee: FuncId(0),
                trap,
            },
            Ty::Unit,
            &[y],
        );
        let max = b.int(I64, Num::from_i128(255));
        let fits = b.inst(Op::Cmp(CmpOp::Le), Ty::Bool, &[y, max]);
        let narrowing = b.trap_site("narrowing", &["out of range"]);
        let fits_u8 = Fact::Within(Num::ZERO, Num::from_i128(255));
        let narrowed = b.guard(narrowing, fits_u8, &[fits, y]);
        let small = b.inst(Op::Cast, Ty::Int(U8), &[narrowed]);
        let r = b.inst(Op::Call(FuncId(0)), Ty::Int(U8), &[small]);
        b.ret(Some(r));
        let m = Module {
            functions: vec![callee, id, b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        cfg::verify(&m).unwrap();
        m
    }

    /// The instructions of each block of function `fi` of `broken`, each by its place in the same
    /// block of `original`, `i0`, `i1` and on, or `new` where it has none there.
    fn order(original: &Module, broken: &Module, fi: usize) -> Vec<String> {
        let before = &original.functions[fi].blocks;
        broken.functions[fi]
            .blocks
            .iter()
            .zip(before)
            .map(|(b, o)| {
                let place = |v: &NodeId| {
                    o.insts
                        .iter()
                        .position(|u| u == v)
                        .map_or("new".to_string(), |i| format!("i{}", i))
                };
                b.insts.iter().map(place).collect::<Vec<_>>().join(" ")
            })
            .collect()
    }

    /// `fn caller(x: i64) -> i64`, which counts and calls `id` on `x`, then counts `id` on what it
    /// gave and calls it in the next block.
    fn counted_across_blocks() -> Module {
        let mut i = Builder::new("id", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = i.params(BlockId(0))[0];
        i.ret(Some(x));
        let mut id = i.finish();
        id.frame = 72;
        let mut b = Builder::new("caller", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = b.params(BlockId(0))[0];
        let next = b.block(&[]);
        let enter = |b: &mut Builder, arg: NodeId| {
            let trap = Some(b.trap_site("stack-overflow", &["the call stack ran out"]));
            b.inst(
                Op::Enter {
                    callee: FuncId(0),
                    trap,
                },
                Ty::Unit,
                &[arg],
            )
        };
        enter(&mut b, x);
        let y = b.inst(Op::Call(FuncId(0)), Ty::Int(I64), &[x]);
        enter(&mut b, y);
        b.jump(next, vec![]);
        b.switch(next);
        let z = b.inst(Op::Call(FuncId(0)), Ty::Int(I64), &[y]);
        b.ret(Some(z));
        let m = Module {
            functions: vec![id, b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        cfg::verify(&m).unwrap();
        m
    }

    /// An `Enter` dropped, doubled, moved past its call, moved past the narrowing check that takes
    /// it, or moved before the call that gives its argument breaks the count of a call's frame
    /// (`core-calculus.md`, item 25), and the verifier refuses each: a count taken where it is not
    /// defined, or a call that does not close the latest count open. Each change stands where its
    /// name says, its call in the count's block or in a later one. The original verifies and runs.
    #[test]
    fn test_the_verifier_refuses_every_count_out_of_its_place() {
        let m = counted_call();
        let all = unpaired(&m);
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "caller/b0.i0/enter-dropped",
                "caller/b0.i0/enter-doubled",
                "caller/b0.i0/enter-moved",
                "caller/b0.i2/enter-dropped",
                "caller/b0.i2/enter-doubled",
                "caller/b0.i2/enter-moved",
                "caller/b0.i2/enter-later",
                "caller/b0.i2/enter-earlier",
                "caller/b0.i1/call-later",
            ]
        );
        for (name, broken) in &all {
            let why = cfg::verify(broken).unwrap_err();
            // The order of values refuses a count or a call taken before it is given; the chain,
            // one that takes an effect not the latest; the walk, a call that closes no count, or a
            // count left open.
            let structural = [
                "is not defined before it on every path",
                "does not close the latest count open",
                "returns with the count",
                "where the latest effect is",
            ];
            assert!(
                structural.iter().any(|r| why.contains(r)),
                "{}: {}",
                name,
                why
            );
        }
        let orders: Vec<Vec<String>> = all.iter().map(|(_, broken)| order(&m, broken, 2)).collect();
        assert_eq!(
            orders,
            [
                ["i1 i2 i3 i4 i5 i6 i7"],
                ["i0 new i1 i2 i3 i4 i5 i6 i7"],
                // Past its call.
                ["i1 i0 i2 i3 i4 i5 i6 i7"],
                ["i0 i1 i3 i4 i5 i6 i7"],
                ["i0 i1 i2 new i3 i4 i5 i6 i7"],
                ["i0 i1 i3 i4 i5 i6 i7 i2"],
                // Past the check that takes it, before the call that gives its argument.
                ["i0 i1 i3 i4 i5 i2 i6 i7"],
                ["i0 i2 i1 i3 i4 i5 i6 i7"],
                // The call of `id` past the call of `callee`.
                ["i0 i2 i3 i4 i5 i6 i7 i1"],
            ]
        );
        let ran = cfg::run(
            &m,
            FuncId(2),
            &[int(7)],
            &mut Memory::default(),
            Limits::default(),
        );
        assert_eq!(ran, Ok(int(7)));
        // A count whose call stands in the next block moves past it there.
        let across = counted_across_blocks();
        let all = unpaired(&across);
        let named: Vec<(&str, Vec<String>)> = all
            .iter()
            .map(|(n, broken)| (n.as_str(), order(&across, broken, 1)))
            .collect();
        let both = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        assert_eq!(
            named,
            [
                ("caller/b0.i0/enter-dropped", both("i1 i2", "i0")),
                ("caller/b0.i0/enter-doubled", both("i0 new i1 i2", "i0")),
                ("caller/b0.i0/enter-moved", both("i1 i0 i2", "i0")),
                ("caller/b0.i2/enter-dropped", both("i0 i1", "i0")),
                ("caller/b0.i2/enter-doubled", both("i0 i1 i2 new", "i0")),
                ("caller/b0.i2/enter-moved", both("i0 i1", "i0 new")),
                ("caller/b0.i2/enter-earlier", both("i0 i2 i1", "i0")),
            ]
        );
        for (name, broken) in &all {
            assert!(cfg::verify(broken).is_err(), "{}", name);
        }
    }

    /// `fn pick(c: bool, x: i64) -> i64`: `(x + 1) * 2`, the sum checked, where `c` holds, else
    /// `x / x` by a divisor a guard holds is not zero, joined, where a comparison takes no effect.
    fn pick() -> cfg::Function {
        let mut b = Builder::new("pick", &[Ty::Bool, Ty::Int(I64)], Some(Ty::Int(I64)));
        let [c, x] = b.params(BlockId(0))[..] else {
            panic!("two parameters")
        };
        let then = b.block(&[]);
        let otherwise = b.block(&[]);
        let join = b.block(&[Ty::Int(I64)]);
        b.branch(c, (then, vec![]), (otherwise, vec![]));
        b.switch(then);
        let one = b.int(I64, Num::from_i128(1));
        let t = b.trap_site("overflow", &["", " does not fit in `i64`"]);
        let up = b.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: Some(t),
            },
            Ty::Int(I64),
            &[x, one],
        );
        let twice = b.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: None,
            },
            Ty::Int(I64),
            &[up, up],
        );
        b.jump(join, vec![twice]);
        b.switch(otherwise);
        let zero = b.int(I64, Num::ZERO);
        let nz = b.inst(Op::Cmp(CmpOp::Ne), Ty::Bool, &[x, zero]);
        let t = b.trap_site("division-by-zero", &["division by zero"]);
        let divisor = b.guard(t, Fact::NonZero, &[nz, x]);
        let q = b.inst(
            Op::Arith {
                op: ArithOp::Div,
                trap: None,
            },
            Ty::Int(I64),
            &[x, divisor],
        );
        b.jump(join, vec![q]);
        b.switch(join);
        let r = b.params(join)[0];
        b.inst(Op::Cmp(CmpOp::Eq), Ty::Bool, &[r, r]);
        b.ret(Some(r));
        b.finish()
    }

    /// Every break of the chain of effects [`unchained`] plants is refused, each kind planted
    /// at least once: in a caller that counts, narrows and calls, and in a function that
    /// branches and joins with a value. The originals verify and run.
    #[test]
    fn test_the_verifier_refuses_every_break_of_the_chain() {
        let mut m = counted_call();
        m.functions.push(pick());
        cfg::verify(&m).unwrap();
        let all = unchained(&m);
        for kind in [
            "effect-dropped",
            "effect-forked",
            "effect-valued",
            "partial-raised",
            "guard-unguarded",
            "guard-raised",
            "passed-valued",
            "ended-early",
        ] {
            assert!(
                all.iter().any(|(n, _)| n.ends_with(kind)),
                "no {} planted",
                kind
            );
        }
        for (name, broken) in &all {
            assert!(cfg::verify(broken).is_err(), "{} verifies", name);
        }
        // Each site and way, and what the moves and the re-pointing leave.
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "caller/b0.i0/effect-dropped",
                "caller/b0.i0/effect-valued",
                "caller/b0.i1/effect-dropped",
                "caller/b0.i1/effect-forked",
                "caller/b0.i1/effect-valued",
                "caller/b0.i2/effect-dropped",
                "caller/b0.i2/effect-forked",
                "caller/b0.i5/effect-dropped",
                "caller/b0.i5/effect-forked",
                "caller/b0.i5/effect-valued",
                "caller/b0.i7/effect-dropped",
                "caller/b0.i7/effect-forked",
                "caller/b0.i7/effect-valued",
                "caller/b0.i6/guard-unguarded",
                "caller/b0.i6/guard-raised",
                "caller/b0.term/ended-early",
                "pick/b0.term/passed-valued",
                "pick/b1.i1/effect-dropped",
                "pick/b1.i1/effect-valued",
                "pick/b1.i2/effect-dropped",
                "pick/b1.i2/partial-raised",
                "pick/b1.term/passed-valued",
                "pick/b2.i2/effect-dropped",
                "pick/b2.i2/effect-valued",
                "pick/b2.i3/guard-unguarded",
                "pick/b2.i3/guard-raised",
                "pick/b2.term/passed-valued",
            ]
        );
        let plant = |name: &str| {
            &all.iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("no plant {}", name))
                .1
        };
        // The exact doubling above the checked sum it reads; the division above its guard.
        assert_eq!(
            order(&m, plant("pick/b1.i2/partial-raised"), 3)[1],
            "i0 i2 i1"
        );
        assert_eq!(
            order(&m, plant("pick/b2.i3/guard-raised"), 3)[2],
            "i0 i1 i3 i2"
        );
        // The division and the cast taking what their guards take.
        let pick = &m.functions[3];
        let (q, x) = (pick.blocks[2].insts[3], pick.blocks[0].params[1]);
        for name in ["pick/b2.i3/guard-unguarded", "pick/b2.i3/guard-raised"] {
            assert_eq!(plant(name).functions[3].own_args(q)[1], x, "{}", name);
        }
        let caller = &m.functions[2];
        let (cast, y) = (caller.blocks[0].insts[6], caller.blocks[0].insts[1]);
        assert_eq!(
            plant("caller/b0.i6/guard-unguarded").functions[2].own_args(cast)[0],
            y
        );
        let ran = |args: &[Val]| {
            cfg::run(
                &m,
                FuncId(3),
                args,
                &mut Memory::default(),
                Limits::default(),
            )
        };
        assert_eq!(ran(&[Val::Bool(true), int(4)]), Ok(int(10)));
        assert_eq!(ran(&[Val::Bool(false), int(4)]), Ok(int(1)));
    }

    fn named<'a>(m: &Module, all: &'a [Mutant], name: &str) -> &'a Mutant {
        all.iter()
            .find(|x| x.name(&m.functions[x.func.0 as usize].name) == name)
            .unwrap_or_else(|| panic!("no mutant {}", name))
    }

    /// A mutant is named by function, block, instruction and operator, changes only a copy,
    /// and the loop's mutants each change what `sum(4)` returns or make it run on forever.
    #[test]
    fn test_mutants_of_a_loop_are_named_and_change_only_a_copy() {
        let m = sum();
        let (all, skipped) = split(&m);
        assert!(skipped.is_empty(), "{:?}", skipped);
        let names: Vec<String> = all.iter().map(|x| x.name("sum")).collect();
        assert_eq!(
            names,
            [
                "sum/b0.i0/int+1",
                "sum/b0.i0/int-1",
                "sum/b1.i0/lt>ge",
                "sum/b1.i0/lt>le",
                "sum/b1.term/swap",
                "sum/b1.term/then",
                "sum/b1.term/else",
                "sum/b2.i0/add>sub",
                "sum/b2.i1/int+1",
                "sum/b2.i1/int-1",
                "sum/b2.i2/add>sub",
            ]
        );
        let original = run(&m, &[int(4)]);
        assert_eq!(original, Ok(int(6)));
        let lt_le = named(&m, &all, "sum/b1.i0/lt>le").apply(&m);
        assert_eq!(run(&lt_le, &[int(4)]), Ok(int(10)));
        assert_eq!(run(&m, &[int(4)]), original, "the original is left alone");
        let swapped = named(&m, &all, "sum/b1.term/swap").apply(&m);
        assert_eq!(run(&swapped, &[int(4)]), Ok(int(0)));
        let never = named(&m, &all, "sum/b1.term/else").apply(&m);
        assert_eq!(run(&never, &[int(4)]), Ok(int(0)));
        let forever = named(&m, &all, "sum/b2.i1/int-1").apply(&m);
        assert_eq!(run(&forever, &[int(4)]), Err(Stop::OutOfSteps));
        // At n = 0 the loop never runs, and `lt>le` would only show at a later input.
        assert_eq!(run(&lt_le, &[int(0)]), Ok(int(0)));
        assert_eq!(
            named(&m, &all, "sum/b2.i0/add>sub").loc(&m),
            cfg::Loc::default()
        );
    }

    /// One function with an instruction of every kind the operators change, and the mutants of
    /// each family, all of which verify but the one that replaces a shift by an operand of
    /// another type.
    #[test]
    fn test_each_operator_family_makes_mutants_that_verify() {
        let mut b = Builder::new(
            "all",
            &[Ty::Int(I64), Ty::Int(I64), Ty::Addr(cfg::MemId(0))],
            Some(Ty::Int(I64)),
        );
        let [x, y, out] = b.params(BlockId(0))[..] else {
            panic!("three parameters")
        };
        b.at(3, 7);
        let checked = b.trap_site("overflow", &["", " leaves its type"]);
        let failed = b.trap_site("check", &["x is ", ""]);
        let bits = b.int(U8, Num::from_i128(3));
        let k = b.int(U8, Num::from_i128(1));
        let shl = b.inst(Op::Shl { trap: checked }, Ty::Int(I64), &[bits, k]);
        let shr = b.inst(Op::Shr, Ty::Int(I64), &[x, k]);
        let mut last = shr;
        let y = b.assume(y, Fact::NonZero);
        for op in [
            ArithOp::Add,
            ArithOp::Sub,
            ArithOp::Mul,
            ArithOp::Div,
            ArithOp::Rem,
        ] {
            last = b.inst(
                Op::Arith {
                    op,
                    trap: Some(checked),
                },
                Ty::Int(I64),
                &[last, y],
            );
        }
        let neg = b.inst(Op::Neg { trap: None }, Ty::Int(I64), &[last]);
        let anded = b.inst(Op::Bits(BitOp::And), Ty::Int(I64), &[neg, shl]);
        let ored = b.inst(Op::Bits(BitOp::Or), Ty::Int(I64), &[anded, x]);
        let xored = b.inst(Op::Bits(BitOp::Xor), Ty::Int(I64), &[ored, x]);
        let flipped = b.inst(Op::Not, Ty::Int(I64), &[xored]);
        let mut conds = Vec::new();
        for c in [
            CmpOp::Eq,
            CmpOp::Ne,
            CmpOp::Lt,
            CmpOp::Le,
            CmpOp::Gt,
            CmpOp::Ge,
        ] {
            conds.push(b.inst(Op::Cmp(c), Ty::Bool, &[x, y]));
        }
        let yes = b.inst(Op::Bool(true), Ty::Bool, &[]);
        let both = b.inst(Op::Bits(BitOp::And), Ty::Bool, &[conds[0], yes]);
        b.inst(Op::Check { trap: failed }, Ty::Unit, &[both, x]);
        b.inst(Op::Store, Ty::Unit, &[out, flipped]);
        let copy = b.inst(Op::Slot { bytes: 8 }, Ty::Addr(cfg::MemId(0)), &[]);
        b.inst(Op::Copy { bytes: 8 }, Ty::Unit, &[copy, out]);
        let then = b.block(&[Ty::Int(I64)]);
        let otherwise = b.block(&[Ty::Int(I64)]);
        b.branch(conds[2], (then, vec![flipped]), (otherwise, vec![x]));
        for t in [then, otherwise] {
            b.switch(t);
            let p = b.params(t)[0];
            b.ret(Some(p));
        }
        let mut mems = cfg::Mems::default();
        mems.of(cfg::Mem::Int(I64));
        let m = Module {
            functions: vec![b.finish()],
            budget: 1 << 20,
            mems,
        };
        cfg::verify(&m).unwrap();

        let (kept, skipped) = split(&m);
        for family in Family::ALL {
            assert!(
                kept.iter().any(|x| x.family == family),
                "no mutant of the family {}",
                family.name()
            );
        }
        for x in &kept {
            assert_eq!(cfg::verify(&x.apply(&m)), Ok(()), "{}", x.name("all"));
            assert_ne!(x.apply(&m), m, "{} changes nothing", x.name("all"));
        }
        // `shl` gives an i64 from a u8: its first operand cannot stand in for it.
        let skipped: Vec<String> = skipped.iter().map(|(x, _)| x.name("all")).collect();
        assert_eq!(skipped, ["all/b0.i2/shl>id"]);
        let operators: Vec<&str> = kept.iter().map(|x| x.operator).collect();
        for expected in [
            "add>sub",
            "sub>add",
            "mul>div",
            "div>rem",
            "rem>div",
            "unchecked",
            "and>or",
            "and>xor",
            "or>and",
            "or>xor",
            "xor>and",
            "xor>or",
            "eq>ne",
            "ne>eq",
            "lt>ge",
            "lt>le",
            "le>gt",
            "le>lt",
            "gt>le",
            "gt>ge",
            "ge>lt",
            "ge>gt",
            "int+1",
            "int-1",
            "int>0",
            "flip",
            "swap",
            "then",
            "else",
            "check-removed",
            "neg>id",
            "not>id",
            "shr>id",
            "store-removed",
            "copy-removed",
        ] {
            assert!(operators.contains(&expected), "no operator {}", expected);
        }
        // Each checked operation also loses its trap; the check reports its line.
        let unchecked = kept.iter().filter(|x| x.operator == "unchecked").count();
        assert_eq!(unchecked, 5);
        let removed = named(&m, &kept, "all/b0.i23/check-removed");
        assert_eq!(removed.loc(&m), cfg::Loc { line: 3, col: 7 });
        assert_eq!(
            removed.apply(&m).functions[0].blocks[0].insts.len(),
            m.functions[0].blocks[0].insts.len() - 1
        );
    }

    /// A check taken out just before a trap hands the trap the effect the check took, so that the
    /// mutant verifies and the trap is reached where the check would have trapped first.
    #[test]
    fn test_a_check_taken_out_hands_a_trap_the_effect_it_took() {
        let mut b = Builder::new("halt", &[Ty::Bool], None);
        let c = b.params(BlockId(0))[0];
        let entry = b.latest();
        let t = b.trap_site("check", &["c is false"]);
        b.inst(Op::Check { trap: t }, Ty::Unit, &[c]);
        let todo = b.trap_site("todo", &["todo reached"]);
        b.trap(todo);
        let m = Module {
            functions: vec![b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        cfg::verify(&m).unwrap();
        let removed = named(&m, &mutants(&m), "halt/b0.i0/check-removed").apply(&m);
        assert_eq!(cfg::verify(&removed), Ok(()));
        assert_eq!(removed.functions[0].blocks[0].term, Term::Trap(todo, entry));
    }

    /// A guard taken out becomes an assumption of what it guarded, in its place: the value it
    /// guarded still flows to the cast that takes it, and where the guard trapped, the run is now
    /// broken MIR, as a check taken out leaves it.
    #[test]
    fn test_a_guard_taken_out_leaves_its_fact_assumed() {
        let m = counted_call();
        let all = mutants(&m);
        let removed = named(&m, &all, "caller/b0.i5/check-removed").apply(&m);
        assert_eq!(cfg::verify(&removed), Ok(()));
        let (before, after) = (&m.functions[2], &removed.functions[2]);
        assert_eq!(before.blocks[0].insts, after.blocks[0].insts);
        let guard = after.blocks[0].insts[5];
        assert!(matches!(after.op(guard), Op::Assume(_)));
        assert_eq!(
            after.fact(guard),
            Some(Fact::Within(Num::ZERO, Num::from_i128(255)))
        );
        let ran = |m: &Module, n: i128| {
            cfg::run(
                m,
                FuncId(2),
                &[int(n)],
                &mut Memory::default(),
                Limits::default(),
            )
        };
        assert_eq!(ran(&removed, 7), Ok(int(7)));
        assert!(matches!(ran(&m, 300), Err(Stop::Trap(_))));
        assert!(matches!(ran(&removed, 300), Err(Stop::Broken(_))));
    }

    /// A constant moves to `c + 1`, `c - 1` and `0` only where its type holds the new value,
    /// and a negation replaced by its operand gives the operand at every use.
    #[test]
    fn test_constants_stay_in_their_type_and_bypasses_rewire_every_use() {
        let consts = |t: IntTy, n: i128| {
            let mut b = Builder::new("k", &[], Some(Ty::Int(t)));
            let k = b.int(t, Num::from_i128(n));
            b.ret(Some(k));
            let m = Module {
                functions: vec![b.finish()],
                budget: 1 << 20,
                mems: cfg::Mems::default(),
            };
            mutants(&m)
                .iter()
                .map(|x| {
                    let out = run(&x.apply(&m), &[]);
                    format!("{}={:?}", x.operator, out)
                })
                .collect::<Vec<_>>()
        };
        let v = |n: i128| Ok::<Val, Stop>(int(n));
        assert_eq!(
            consts(U8, 0),
            [format!("int+1={:?}", v(1))],
            "0 - 1 is no u8, and 0 is already 0"
        );
        assert_eq!(
            consts(U8, 255),
            [format!("int-1={:?}", v(254)), format!("int>0={:?}", v(0))]
        );
        assert_eq!(
            consts(I64, -1),
            [format!("int+1={:?}", v(0)), format!("int-1={:?}", v(-2)),],
            "-1 + 1 is already 0"
        );
        assert_eq!(step(Num::new(false, u128::MAX), true), None);
        assert_eq!(step(Num::ZERO, false), Some(Num::from_i128(-1)));

        // `fn n(x: i64) -> i64 { val y = -x; if y < 0 { y } else { y + 1 } }`
        let mut b = Builder::new("n", &[Ty::Int(I64)], Some(Ty::Int(I64)));
        let x = b.params(BlockId(0))[0];
        let y = b.inst(Op::Neg { trap: None }, Ty::Int(I64), &[x]);
        let zero = b.int(I64, Num::ZERO);
        b.at(5, 9);
        let below = b.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[y, zero]);
        b.at(6, 2);
        let one = b.int(I64, Num::from_i128(1));
        let up = b.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: None,
            },
            Ty::Int(I64),
            &[y, one],
        );
        let join = b.block(&[Ty::Int(I64)]);
        let latest = b.latest();
        b.branch(below, (join, vec![y]), (join, vec![up]));
        b.switch(join);
        let r = b.params(join)[0];
        b.ret(Some(r));
        let m = Module {
            functions: vec![b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        let all = mutants(&m);
        let id = named(&m, &all, "n/b0.i0/neg>id").apply(&m);
        assert_eq!(cfg::verify(&id), Ok(()));
        assert_eq!(run(&m, &[int(5)]), Ok(int(-5)));
        assert_eq!(run(&id, &[int(5)]), Ok(int(6)));
        assert_eq!(run(&id, &[int(-5)]), Ok(int(-5)));
        assert_eq!(
            named(&m, &all, "n/b0.term/then").apply(&m).functions[0].blocks[0].term,
            Term::Jump(join, vec![y, latest])
        );
        assert_eq!(
            named(&m, &all, "n/b0.term/else").apply(&m).functions[0].blocks[0].term,
            Term::Jump(join, vec![up, latest])
        );
        // A `not` replaced by its operand: the branch it decides reads the operand, and the effect
        // the branch passes on stays the latest.
        let mut b = Builder::new("flip", &[Ty::Bool], Some(Ty::Int(I64)));
        let c = b.params(BlockId(0))[0];
        let not = b.inst(Op::Not, Ty::Bool, &[c]);
        let (yes, no) = (b.block(&[]), b.block(&[]));
        let entry = b.latest();
        b.branch(not, (yes, vec![]), (no, vec![]));
        for (block, n) in [(yes, 1), (no, 0)] {
            b.switch(block);
            let k = b.int(I64, Num::from_i128(n));
            b.ret(Some(k));
        }
        let flip = Module {
            functions: vec![b.finish()],
            budget: 1 << 20,
            mems: cfg::Mems::default(),
        };
        let id = named(&flip, &mutants(&flip), "flip/b0.i0/not>id").apply(&flip);
        assert_eq!(cfg::verify(&id), Ok(()));
        assert_eq!(
            id.functions[0].blocks[0].term,
            Term::Branch {
                cond: c,
                then: (yes, vec![entry]),
                otherwise: (no, vec![entry]),
            }
        );
        // A branch's mutant is located at its condition, an instruction's at the instruction.
        let swap = named(&m, &all, "n/b0.term/swap");
        assert_eq!(swap.loc(&m), cfg::Loc { line: 5, col: 9 });
        assert_eq!(swap.value(&m), None);
        let lt = named(&m, &all, "n/b0.i2/lt>ge");
        assert_eq!(
            (lt.value(&m), lt.loc(&m)),
            (Some(below), cfg::Loc { line: 5, col: 9 })
        );
        let int = named(&m, &all, "n/b0.i3/int+1");
        assert_eq!(
            (int.value(&m), int.loc(&m)),
            (Some(one), cfg::Loc { line: 6, col: 2 })
        );
        // A terminator that is no branch, or a function the module lacks, has no place.
        let at_return = Mutant {
            block: join,
            ..swap.clone()
        };
        assert_eq!(at_return.loc(&m), cfg::Loc::default());
        assert_eq!(at_return.apply(&m), m);
        let elsewhere = Mutant {
            func: FuncId(7),
            ..lt.clone()
        };
        assert_eq!(
            (elsewhere.value(&m), elsewhere.loc(&m)),
            (None, cfg::Loc::default())
        );
        assert_eq!(elsewhere.apply(&m), m);
    }
}

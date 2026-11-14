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
//!   operation made exact, so that where it would trap the MIR interpreter reports broken MIR;
//! - [`Family::Identity`]: a negation, a `not` or a shift replaced by its first operand;
//! - [`Family::Memory`]: a store or a copy taken out.
//!
//! Not mutated: a cast, which only moves a value between types that hold it; a slot, an element
//! address or a load, whose change reads other bytes of the same frame rather than another value
//! the program computes; a call; and the terminators `jump`, `return` and `trap`, whose targets
//! and values the other families already reach through the instructions that compute them.
//!
//! A mutant is named by its function, its block, the instruction's index in that block or
//! `term`, and its operator: `clamp/b2.i3/lt>le`. The name stays the same through any change to
//! another function and to the instructions after it in its block, so a record that a mutant
//! is equivalent survives unrelated edits as far as an index can.

use crate::cfg::{self, ArithOp, BitOp, BlockId, CmpOp, FuncId, Module, Num, Op, Term, Ty};
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
                    Change::Op(op) => f.ops[v.0 as usize] = op.clone(),
                    Change::Remove => {
                        block.insts.remove(i as usize);
                    }
                    Change::Bypass => {
                        let Some(&first) = f.args(v).first() else {
                            return out;
                        };
                        replace_uses(f, v, first);
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

/// Every use of `old` in `f`, as an operand or in a terminator, reads `new` instead.
fn replace_uses(f: &mut cfg::Function, old: NodeId, new: NodeId) {
    for p in f.pool.iter_mut().filter(|p| **p == old) {
        *p = new;
    }
    let swap = |v: &mut NodeId| {
        if *v == old {
            *v = new;
        }
    };
    for b in &mut f.blocks {
        match &mut b.term {
            Term::Jump(_, args) => args.iter_mut().for_each(swap),
            Term::Branch {
                cond,
                then,
                otherwise,
            } => {
                swap(cond);
                then.1.iter_mut().for_each(swap);
                otherwise.1.iter_mut().for_each(swap);
            }
            Term::Return(Some(v)) => swap(v),
            Term::Return(None) | Term::Trap(_) => {}
        }
    }
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
        Op::Check { .. } => vec![(Family::Trap, "check-removed", Change::Remove)],
        Op::Store => vec![(Family::Memory, "store-removed", Change::Remove)],
        Op::Copy { .. } => vec![(Family::Memory, "copy-removed", Change::Remove)],
        Op::Param | Op::Cast | Op::Slot { .. } | Op::Elem { .. } | Op::Load | Op::Call(_) => {
            Vec::new()
        }
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
    use crate::cfg::{Builder, IntTy, Limits, Memory, Stop, Val};

    const I64: IntTy = IntTy::new(64, true);
    const U8: IntTy = IntTy::new(8, false);

    fn run(m: &Module, args: &[Val]) -> Result<Val, Stop> {
        cfg::run(
            m,
            FuncId(0),
            args,
            &mut Memory::default(),
            Limits {
                max_depth: 100,
                max_steps: 10_000,
            },
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
        let add = Op::Arith {
            op: ArithOp::Add,
            trap: None,
        };
        let s2 = b.inst(add.clone(), Ty::Int(I64), &[s, i]);
        let one = b.int(I64, Num::from_i128(1));
        let i2 = b.inst(add, Ty::Int(I64), &[i, one]);
        b.end(Term::Jump(head, vec![i2, s2]));
        b.switch(exit);
        b.end(Term::Return(Some(s)));
        let m = Module {
            functions: vec![b.finish()],
        };
        cfg::verify(&m).unwrap();
        m
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
            &[Ty::Int(I64), Ty::Int(I64), Ty::Addr],
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
        let copy = b.inst(Op::Slot { bytes: 8 }, Ty::Addr, &[]);
        b.inst(Op::Copy { bytes: 8 }, Ty::Unit, &[copy, out]);
        let then = b.block(&[Ty::Int(I64)]);
        let otherwise = b.block(&[Ty::Int(I64)]);
        b.end(Term::Branch {
            cond: conds[2],
            then: (then, vec![flipped]),
            otherwise: (otherwise, vec![x]),
        });
        for t in [then, otherwise] {
            b.switch(t);
            let p = b.params(t)[0];
            b.end(Term::Return(Some(p)));
        }
        let m = Module {
            functions: vec![b.finish()],
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
        let removed = named(&m, &kept, "all/b0.i22/check-removed");
        assert_eq!(removed.loc(&m), cfg::Loc { line: 3, col: 7 });
        assert_eq!(
            removed.apply(&m).functions[0].blocks[0].insts.len(),
            m.functions[0].blocks[0].insts.len() - 1
        );
    }

    /// A constant moves to `c + 1`, `c - 1` and `0` only where its type holds the new value,
    /// and a negation replaced by its operand gives the operand at every use.
    #[test]
    fn test_constants_stay_in_their_type_and_bypasses_rewire_every_use() {
        let consts = |t: IntTy, n: i128| {
            let mut b = Builder::new("k", &[], Some(Ty::Int(t)));
            let k = b.int(t, Num::from_i128(n));
            b.end(Term::Return(Some(k)));
            let m = Module {
                functions: vec![b.finish()],
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
        b.end(Term::Branch {
            cond: below,
            then: (join, vec![y]),
            otherwise: (join, vec![up]),
        });
        b.switch(join);
        let r = b.params(join)[0];
        b.end(Term::Return(Some(r)));
        let m = Module {
            functions: vec![b.finish()],
        };
        let all = mutants(&m);
        let id = named(&m, &all, "n/b0.i0/neg>id").apply(&m);
        assert_eq!(cfg::verify(&id), Ok(()));
        assert_eq!(run(&m, &[int(5)]), Ok(int(-5)));
        assert_eq!(run(&id, &[int(5)]), Ok(int(6)));
        assert_eq!(run(&id, &[int(-5)]), Ok(int(-5)));
        assert_eq!(
            named(&m, &all, "n/b0.term/then").apply(&m).functions[0].blocks[0].term,
            Term::Jump(join, vec![y])
        );
        assert_eq!(
            named(&m, &all, "n/b0.term/else").apply(&m).functions[0].blocks[0].term,
            Term::Jump(join, vec![up])
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

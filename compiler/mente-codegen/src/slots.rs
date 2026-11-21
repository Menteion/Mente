//! Where native code lays each stack slot of a MIR function (`spec/mir-and-codegen.md`, item 5,
//! "Slots given by scope and reused").
//!
//! MIR makes a slot where its `slot` instruction runs, so a slot of a branch not taken takes
//! nothing, and a frame counts one branch of an `if` or one arm of a `match`
//! (`core-calculus.md`, item 25). Cranelift merges none of the stack slots it is given, so one
//! Cranelift slot for each MIR slot made a frame the sum of every slot in the function. Here the
//! slots that are never alive together share bytes, in one Cranelift slot.
//!
//! A slot is alive where an address into it is: MIR keeps addresses only in values, never in
//! memory (a store or a load of an address is broken MIR), so its bytes are reached only through
//! the slot's own value and the values derived from it, element addresses and the block
//! parameters they are passed to. Where none of those is alive, nothing can read or write the
//! slot, and another may take its bytes.

use std::collections::BTreeSet;

use mente_mir::cfg::{self, Op, Term};
use mente_mir::NodeId;

/// The alignment of every slot, which `Op::Slot` promises.
const ALIGN: u32 = 16;

/// The bytes of a frame left beside a function's slots for the one slot native code adds, its
/// scratch (`native::SCRATCH`).
const RESERVED: u32 = crate::native::SCRATCH;

/// The bytes of one function's slots: the offset of each `slot` instruction, by value, within
/// one area of `bytes`.
pub(crate) struct Layout {
    pub(crate) offsets: Vec<Option<u32>>,
    pub(crate) bytes: u32,
}

/// The bytes a `slot` instruction makes, if `op` is one.
fn slot_bytes(op: &Op) -> Option<u32> {
    match op {
        Op::Slot { bytes } => Some(*bytes),
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
        | Op::Elem { .. }
        | Op::Load
        | Op::Store
        | Op::Copy { .. }
        | Op::Call(_)
        | Op::Enter { .. } => None,
    }
}

/// The blocks `term` goes to, each with the values it passes to the block's parameters.
fn targets(term: &Term) -> Vec<(cfg::BlockId, &[NodeId])> {
    match term {
        Term::Jump(t, args) => vec![(*t, args.as_slice())],
        Term::Branch {
            then, otherwise, ..
        } => vec![
            (then.0, then.1.as_slice()),
            (otherwise.0, otherwise.1.as_slice()),
        ],
        Term::Return(..) | Term::Trap(..) => Vec::new(),
    }
}

/// The values `term` reads.
fn term_uses(term: &Term) -> Vec<NodeId> {
    match term {
        Term::Jump(_, args) => args.clone(),
        Term::Branch {
            cond,
            then,
            otherwise,
        } => std::iter::once(*cond)
            .chain(then.1.iter().copied())
            .chain(otherwise.1.iter().copied())
            .collect(),
        Term::Return(value, effect) => value.iter().copied().chain([*effect]).collect(),
        Term::Trap(_, effect) => vec![*effect],
    }
}

/// Where each slot of `f` lies: slots alive at one point lie apart, and the rest share bytes,
/// each placed, in the order the function makes them, at the lowest offset no slot alive with it
/// has taken.
pub(crate) fn layout(f: &cfg::Function) -> Layout {
    let values = f.ops.len();
    let mut offsets = vec![None; values];
    // The slots, in the order of the blocks and their instructions, each of its bytes rounded up
    // to the alignment, at least one.
    let mut slot_of: Vec<Option<usize>> = vec![None; values];
    let mut sizes: Vec<u32> = Vec::new();
    let mut makers: Vec<NodeId> = Vec::new();
    for block in &f.blocks {
        for &v in &block.insts {
            if let Some(bytes) = slot_bytes(&f.ops[v.0 as usize]) {
                slot_of[v.0 as usize] = Some(sizes.len());
                sizes.push(bytes.max(1).div_ceil(ALIGN) * ALIGN);
                makers.push(v);
            }
        }
    }
    if sizes.is_empty() {
        return Layout { offsets, bytes: 0 };
    }
    // The slots each address may point into: a slot's own value its slot; an address computed
    // from addresses, those addresses' slots; a block parameter, the slots of every value passed
    // to it. A parameter of the entry is the caller's memory, in no slot of this function.
    let mut roots: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); values];
    for (k, v) in makers.iter().enumerate() {
        roots[v.0 as usize].insert(k);
    }
    let address = |v: NodeId| f.ty(v) == cfg::Ty::Addr;
    let mut changed = true;
    while changed {
        changed = false;
        for block in &f.blocks {
            for &v in &block.insts {
                if !address(v) || slot_of[v.0 as usize].is_some() {
                    continue;
                }
                let from: BTreeSet<usize> = f
                    .args(v)
                    .iter()
                    .filter(|a| address(**a))
                    .flat_map(|a| roots[a.0 as usize].iter().copied())
                    .collect();
                if !from.is_subset(&roots[v.0 as usize]) {
                    roots[v.0 as usize].extend(from);
                    changed = true;
                }
            }
            for (target, args) in targets(&block.term) {
                for (arg, param) in args.iter().zip(&f.blocks[target.0 as usize].params) {
                    let from = roots[arg.0 as usize].clone();
                    if !from.is_subset(&roots[param.0 as usize]) {
                        roots[param.0 as usize].extend(from);
                        changed = true;
                    }
                }
            }
        }
    }
    let rooted = |v: &NodeId| !roots[v.0 as usize].is_empty();
    // Which addresses are alive where each block begins, by the usual backward flow: alive
    // before an instruction are the addresses it reads and those alive after it but the one it
    // defines; a block's parameters are defined where it begins.
    let live_after = |block: &cfg::Block, live_in: &[BTreeSet<NodeId>]| -> BTreeSet<NodeId> {
        let mut live: BTreeSet<NodeId> = term_uses(&block.term)
            .into_iter()
            .filter(|v| rooted(v))
            .collect();
        for (target, _) in targets(&block.term) {
            live.extend(live_in[target.0 as usize].iter().copied());
        }
        live
    };
    let mut live_in: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); f.blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for (b, block) in f.blocks.iter().enumerate().rev() {
            let mut live = live_after(block, &live_in);
            for &v in block.insts.iter().rev() {
                live.remove(&v);
                live.extend(f.args(v).iter().copied().filter(|a| rooted(a)));
            }
            for p in &block.params {
                live.remove(p);
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
        }
    }
    // Each address's base: the slot or the block parameter it was computed from, element by
    // element. Every address of one base points into one slot, whichever it is in a run, so the
    // slots a base may point into need no bytes apart for its sake: a parameter that may take any
    // of a thousand slots, one on each path, holds one of them.
    // A slot has no operand, so it stays its own base; a value that is no address takes the base
    // of the one address it reads, which nothing asks for, since only addresses have roots.
    let mut base: Vec<NodeId> = (0..values).map(|v| NodeId(v as u32)).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for block in &f.blocks {
            for &v in &block.insts {
                let mut from = f
                    .args(v)
                    .iter()
                    .filter(|a| address(**a))
                    .map(|a| base[a.0 as usize]);
                if let (Some(only), None) = (from.next(), from.next()) {
                    if base[v.0 as usize] != only {
                        base[v.0 as usize] = only;
                        changed = true;
                    }
                }
            }
        }
    }
    // The slots alive together at each point: two slots that two different bases alive there may
    // point into, a base being alive where an address of it is, an operand of the instruction or
    // the value it defines, which takes its slot's bytes where it is made even if nothing reads
    // it. A point whose bases are those of the point before adds nothing.
    let mut together: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); sizes.len()];
    let mut last: Option<BTreeSet<NodeId>> = None;
    let mut meet = |vs: &mut dyn Iterator<Item = NodeId>| {
        let bases: BTreeSet<NodeId> = vs
            .filter(|v| rooted(v))
            .map(|v| base[v.0 as usize])
            .collect();
        if last.as_ref() == Some(&bases) {
            return;
        }
        for &a in &bases {
            for &b in bases.range(NodeId(a.0 + 1)..) {
                for &x in &roots[a.0 as usize] {
                    for &y in &roots[b.0 as usize] {
                        if x != y {
                            together[x].insert(y);
                            together[y].insert(x);
                        }
                    }
                }
            }
        }
        last = Some(bases);
    };
    for block in &f.blocks {
        let mut live = live_after(block, &live_in);
        meet(&mut live.iter().copied());
        for &v in block.insts.iter().rev() {
            meet(
                &mut live
                    .iter()
                    .copied()
                    .chain([v])
                    .chain(f.args(v).iter().copied()),
            );
            live.remove(&v);
            live.extend(f.args(v).iter().copied().filter(|a| rooted(a)));
        }
        meet(&mut live.iter().copied().chain(block.params.iter().copied()));
    }
    // Two slots of which no run makes both, as those of the two branches of an `if`, never hold
    // bytes at once, though an address that may point into either is alive where the branches
    // join. A run that made both would have passed from the one made first to the other, so
    // they share bytes unless one's `slot` instruction reaches the other's.
    let mut where_made: Vec<(usize, usize)> = vec![(0, 0); sizes.len()];
    for (b, block) in f.blocks.iter().enumerate() {
        for (i, v) in block.insts.iter().enumerate() {
            if let Some(k) = slot_of[v.0 as usize] {
                where_made[k] = (b, i);
            }
        }
    }
    let mut reached: Vec<Option<Vec<bool>>> = vec![None; f.blocks.len()];
    // The blocks one jump or more from block `from`.
    let mut reaches = |from: usize, to: usize| -> bool {
        let blocks = reached[from].get_or_insert_with(|| {
            // The blocks one jump or more from `from`'s block.
            let mut seen = vec![false; f.blocks.len()];
            let mut stack: Vec<usize> = targets(&f.blocks[from].term)
                .iter()
                .map(|(t, _)| t.0 as usize)
                .collect();
            while let Some(b) = stack.pop() {
                if !seen[b] {
                    seen[b] = true;
                    stack.extend(targets(&f.blocks[b].term).iter().map(|(t, _)| t.0 as usize));
                }
            }
            seen
        });
        blocks[to]
    };
    // Which pairs of slots one run may both make: two of one block, or two of blocks one of
    // which reaches the other. Two that are alive together in one run lie apart, and two of
    // which no run makes both may share every byte.
    let mut related: Vec<Vec<usize>> = vec![Vec::new(); sizes.len()];
    for k in 0..sizes.len() {
        for o in 0..k {
            let (a, b) = (where_made[o].0, where_made[k].0);
            if a == b || reaches(a, b) || reaches(b, a) {
                related[k].push(o);
            }
        }
    }
    // Two slots one run makes in turn, at one offset, would have one address, which Cranelift
    // computes once and keeps alive from the first slot's use to the last's, so that its register
    // allocation, and the instructions it executes, change with what shares bytes. So a slot
    // starts where no slot of its run started, sharing their bytes from 16 on, while that keeps
    // the slots within the frame they hold values of; past that, at the lowest clear offset.
    let budget = u32::try_from(f.frame.saturating_sub(u128::from(RESERVED))).unwrap_or(u32::MAX);
    let place = |apart: bool| -> (Vec<u32>, u32) {
        let mut placed: Vec<u32> = Vec::with_capacity(sizes.len());
        let mut bytes = 0;
        for k in 0..sizes.len() {
            let taken: Vec<(u32, u32)> = together[k]
                .iter()
                .filter(|o| related[k].binary_search(o).is_ok())
                .map(|&o| (placed[o], placed[o] + sizes[o]))
                .collect();
            let starts: BTreeSet<u32> = if apart {
                related[k].iter().map(|&o| placed[o]).collect()
            } else {
                BTreeSet::new()
            };
            let clear = |at: u32| taken.iter().all(|&(s, e)| at + sizes[k] <= s || e <= at);
            // Where a slot may begin: the start, the end of every slot it lies apart from, and
            // one alignment past each start it does not take.
            let candidates: BTreeSet<u32> = std::iter::once(0)
                .chain(taken.iter().map(|&(_, e)| e))
                .chain(starts.iter().map(|s| s + ALIGN))
                .collect();
            // The last candidate lies past every slot apart and past every start, so one is found.
            let at = candidates
                .into_iter()
                .find(|&at| clear(at) && !starts.contains(&at))
                .unwrap_or(0);
            placed.push(at);
            bytes = bytes.max(at + sizes[k]);
        }
        (placed, bytes)
    };
    let (placed, bytes) = match place(true) {
        (placed, bytes) if bytes <= budget => (placed, bytes),
        _ => place(false),
    };
    for (k, at) in placed.into_iter().enumerate() {
        offsets[makers[k].0 as usize] = Some(at);
    }
    Layout { offsets, bytes }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout of the function named `name` in `source`, with the bytes of each slot in the
    /// order the function makes them.
    fn layout_of(source: &str, name: &str) -> (Layout, Vec<(u32, u32)>) {
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        let f = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap();
        let layout = layout(f);
        let slots = f
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .filter_map(|v| {
                let bytes = slot_bytes(&f.ops[v.0 as usize])?;
                Some((layout.offsets[v.0 as usize].unwrap(), bytes))
            })
            .collect();
        (layout, slots)
    }

    /// Two slots one run may both make share bytes when one block parameter may take either and
    /// nothing else holds the first once the second is made: a parameter holds one slot at a
    /// time. Counting every slot a parameter may take as alive at once gave a generated program's
    /// parameter, joined from three hundred paths, a thousand slots apart, 16,208 bytes for a
    /// frame of 363, which CI's build of it refused (seed 93 of the generator, 2026-11-21).
    #[test]
    fn test_the_slots_one_parameter_may_take_share_bytes() {
        use mente_mir::cfg::{BlockId, Builder, IntTy, Num, Ty};
        let u8t = Ty::Int(IntTy::new(8, false));
        let mut b = Builder::new("f", &[Ty::Bool], Some(u8t));
        let c = b.params(BlockId(0))[0];
        let first = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        let later = b.block(&[]);
        let join = b.block(&[Ty::Addr]);
        b.branch(c, (later, vec![]), (join, vec![first]));
        b.switch(later);
        let second = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.jump(join, vec![second]);
        b.switch(join);
        let p = b.params(join)[0];
        // An element of the parameter, alive beside it, has its base: it points into the one
        // slot the parameter holds, not into a second.
        let one = b.int(IntTy::new(64, false), Num::new(false, 1));
        let at = b.inst(Op::Elem { stride: 1 }, Ty::Addr, &[p, one]);
        let x = b.inst(Op::Load, u8t, &[at]);
        let y = b.inst(Op::Load, u8t, &[p]);
        let v = b.inst(Op::Bits(mente_mir::cfg::BitOp::Or), u8t, &[x, y]);
        b.ret(Some(v));
        let l = layout(&b.finish());
        assert_eq!(l.offsets[first.0 as usize], l.offsets[second.0 as usize]);
        assert_eq!(l.bytes, 16);
    }

    /// Slots one run may make both, two of one block or two of blocks one reaches, start at
    /// offsets of their own while the frame holds them, and share bytes from 16 on, so that
    /// Cranelift never takes two for one address; slots of which no run makes both, those of two
    /// branches, start at one. With no room in the frame, they share their bytes from the start.
    #[test]
    fn test_slots_one_run_makes_start_apart_within_the_frame() {
        use mente_mir::cfg::{BlockId, Builder, IntTy, Ty};
        let u8t = Ty::Int(IntTy::new(8, false));
        let build = |frame: u128| {
            let mut b = Builder::new("f", &[Ty::Bool, u8t], None);
            let [c, x] = b.params(BlockId(0))[..] else {
                panic!("two parameters")
            };
            // Two slots of the entry block, the first dead before the second is made.
            let s1 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
            b.inst(Op::Store, Ty::Unit, &[s1, x]);
            let s2 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
            b.inst(Op::Store, Ty::Unit, &[s2, x]);
            // A slot in each branch, one made after an instruction more than the other.
            let (yes, no) = (b.block(&[]), b.block(&[]));
            b.branch(c, (yes, vec![]), (no, vec![]));
            b.switch(yes);
            let s3 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
            b.inst(Op::Store, Ty::Unit, &[s3, x]);
            b.ret(None);
            b.switch(no);
            let y = b.inst(Op::Not, u8t, &[x]);
            let s4 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
            b.inst(Op::Store, Ty::Unit, &[s4, y]);
            b.ret(None);
            let mut f = b.finish();
            f.frame = frame;
            let l = layout(&f);
            let at = |s: NodeId| l.offsets[s.0 as usize].unwrap_or(u32::MAX);
            ([at(s1), at(s2), at(s3), at(s4)], l.bytes)
        };
        assert_eq!(build(1000), ([0, 16, 32, 32], 48));
        assert_eq!(build(0), ([0, 0, 0, 0], 16));
    }

    /// Whether two slots, each an offset and its bytes, hold no byte in common.
    fn apart(a: (u32, u32), b: (u32, u32)) -> bool {
        a.0 + a.1 <= b.0 || b.0 + b.1 <= a.0
    }

    /// The slots of branches that never run together share bytes: two hundred branches, each of
    /// a 4 KiB array and the binding it initializes, take what one branch does, 8 KiB, where a
    /// slot each took 1.6 MB.
    #[test]
    fn test_slots_of_branches_share_bytes() {
        let mut source = "module m\npub fn f(x: u8) -> u8 {\n".to_string();
        for k in 0..200 {
            source.push_str(&format!(
                "  if x == 7 {{\n    val a{k}: [4096]u8 = [x; 4096]\n    _ = a{k}\n  }}\n"
            ));
        }
        source.push_str("  x\n}\n");
        let (layout, slots) = layout_of(&source, "f");
        assert_eq!(slots.len(), 400, "{slots:?}");
        assert!(slots.iter().all(|(at, _)| *at % 4096 == 0), "{slots:?}");
        assert_eq!(layout.bytes, 2 * 4096);
    }

    /// Slots alive together lie apart, each at its alignment, and a slot whose value has been
    /// copied away gives its bytes to the next: `a`'s literal, copied into `a`, gives its bytes
    /// to `b`'s, while `a` stays apart from both of `b`'s slots.
    #[test]
    fn test_slots_alive_together_lie_apart() {
        let source = "module m\npub fn f(x: u8) -> u8 {\n  val a: [3]u8 = [x; 3]\n  val b: [5]u8 = [x; 5]\n  a[0] + b[0]\n}\n";
        let (layout, slots) = layout_of(source, "f");
        let [a_literal, a, b_literal, b] = slots[..] else {
            panic!("{slots:?}");
        };
        for (x, y) in [(a_literal, a), (a, b_literal), (a, b), (b_literal, b)] {
            assert!(apart(x, y), "{slots:?}");
        }
        assert_eq!(a_literal.0, b_literal.0, "{slots:?}");
        assert!(slots.iter().all(|(at, _)| at % ALIGN == 0), "{slots:?}");
        assert_eq!(layout.bytes, 3 * ALIGN);
    }

    /// The array an `if` gives stays alive where the branches join, in whichever branch's slot
    /// made it, apart from the binding it is copied into; the two branches' slots, of which no
    /// run makes both, share bytes.
    #[test]
    fn test_a_slot_passed_to_a_join_stays_alive() {
        let source = "module m\npub fn f(x: u8, y: u8, c: bool) -> u8 {\n  val a: [4]u8 = if c { [x; 4] } else { [y; 4] }\n  val b: [4]u8 = [x; 4]\n  a[1] + b[2]\n}\n";
        let (layout, slots) = layout_of(source, "f");
        let [then, otherwise, a, b_literal, b] = slots[..] else {
            panic!("{slots:?}");
        };
        assert_eq!(then.0, otherwise.0, "{slots:?}");
        for (x, y) in [
            (then, a),
            (otherwise, a),
            (a, b_literal),
            (a, b),
            (b_literal, b),
        ] {
            assert!(apart(x, y), "{slots:?}");
        }
        assert_eq!(layout.bytes, 3 * ALIGN);
    }
}

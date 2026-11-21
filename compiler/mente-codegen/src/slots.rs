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

/// The alignment a slot of `bytes` takes: the largest power of two up to 16 that divides its size.
/// A value's size is a multiple of its alignment, a power of two up to an `i128`'s 16, so this is
/// at least the alignment of what the slot holds, and no slot is rounded up past its bytes.
fn align_of(bytes: u32) -> u64 {
    match bytes {
        0 => 1,
        n => (1u64 << n.trailing_zeros()).min(16),
    }
}

/// `at` rounded up to a multiple of `align`, a power of two.
fn align_up(at: u64, align: u64) -> u64 {
    at.div_ceil(align) * align
}

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
/// each placed, in the order the function makes them, at the lowest offset of its alignment no
/// slot alive with it has taken. Slots past what one stack slot can hold, 4 GiB, are a limit of
/// the compiler, reported as one.
pub(crate) fn layout(f: &cfg::Function) -> Result<Layout, String> {
    let values = f.ops.len();
    let mut offsets = vec![None; values];
    // The slots, in the order of the blocks and their instructions, each with its bytes, one at
    // least, so that a slot of no bytes has an address in the area as any slot has, and its
    // alignment.
    let mut slot_of: Vec<Option<usize>> = vec![None; values];
    let mut sizes: Vec<u64> = Vec::new();
    let mut aligns: Vec<u64> = Vec::new();
    let mut makers: Vec<NodeId> = Vec::new();
    for block in &f.blocks {
        for &v in &block.insts {
            if let Some(bytes) = slot_bytes(&f.ops[v.0 as usize]) {
                slot_of[v.0 as usize] = Some(sizes.len());
                sizes.push(u64::from(bytes.max(1)));
                aligns.push(align_of(bytes));
                makers.push(v);
            }
        }
    }
    if sizes.is_empty() {
        return Ok(Layout { offsets, bytes: 0 });
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
    // starts where no slot of its run started, at the next offset of its alignment past one, while
    // that keeps the slots within the frame they hold values of; past that, at the lowest clear
    // offset. Offsets and ends are counted in 64 bits, which no sum of 32-bit sizes passes.
    let budget = u64::try_from(f.frame.saturating_sub(u128::from(RESERVED))).unwrap_or(u64::MAX);
    let place = |apart: bool| -> (Vec<u64>, u64) {
        let mut placed: Vec<u64> = Vec::with_capacity(sizes.len());
        let mut bytes = 0;
        for k in 0..sizes.len() {
            let taken: Vec<(u64, u64)> = together[k]
                .iter()
                .filter(|o| related[k].binary_search(o).is_ok())
                .map(|&o| (placed[o], placed[o] + sizes[o]))
                .collect();
            let starts: BTreeSet<u64> = if apart {
                related[k].iter().map(|&o| placed[o]).collect()
            } else {
                BTreeSet::new()
            };
            let clear = |at: u64| taken.iter().all(|&(s, e)| at + sizes[k] <= s || e <= at);
            // Where a slot may begin, at its alignment: the start, the end of every slot it lies
            // apart from, and past each start it does not take.
            let a = aligns[k];
            let candidates: BTreeSet<u64> = std::iter::once(0)
                .chain(taken.iter().map(|&(_, e)| align_up(e, a)))
                .chain(starts.iter().map(|s| align_up(s + 1, a)))
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
    let too_many = || {
        format!(
            "the slots of `{}` take {} bytes, past what a stack slot holds",
            f.name, bytes
        )
    };
    let bytes = u32::try_from(bytes).map_err(|_| too_many())?;
    for (k, at) in placed.into_iter().enumerate() {
        offsets[makers[k].0 as usize] = Some(u32::try_from(at).map_err(|_| too_many())?);
    }
    Ok(Layout { offsets, bytes })
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
        let layout = layout(f).unwrap();
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
        let l = layout(&b.finish()).unwrap();
        assert_eq!(l.offsets[first.0 as usize], l.offsets[second.0 as usize]);
        assert_eq!(l.bytes, 16);
    }

    /// Slots one run may make both, two of one block or two of blocks one reaches, start at
    /// offsets of their own while the frame holds them, the next of their alignment, so that
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
            let l = layout(&f).unwrap();
            let at = |s: NodeId| l.offsets[s.0 as usize].unwrap_or(u32::MAX);
            ([at(s1), at(s2), at(s3), at(s4)], l.bytes)
        };
        assert_eq!(build(1000), ([0, 16, 32, 32], 48));
        assert_eq!(build(0), ([0, 0, 0, 0], 16));
    }

    /// With no room in the frame, slots share bytes wherever they are not alive together, so
    /// what keeps a slot alive decides: an address computed from it, alive past the next slot's
    /// making, and an address a jump passes to the block it joins, read there, each keep it
    /// apart from that slot; two slots of which the first is dead share bytes. While the frame
    /// had room, each slot of a run started at an offset of its own anyway, so that a liveness
    /// that dropped the addresses computed from slots, or the values a jump passes, went unseen
    /// (the mutants of pull request 183 that made `address` true of every other value and
    /// `term_uses` give nothing, 2026-11-22).
    #[test]
    fn test_with_no_room_only_slots_dead_before_the_next_share_bytes() {
        use mente_mir::cfg::{BlockId, Builder, IntTy, Num, Ty};
        let u8t = Ty::Int(IntTy::new(8, false));
        let mut b = Builder::new("f", &[u8t], Some(u8t));
        let x = b.params(BlockId(0))[0];
        // A slot alive past the next one's making through an element of it.
        let s1 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        let zero = b.int(IntTy::new(64, false), Num::new(false, 0));
        let e1 = b.inst(Op::Elem { stride: 1 }, Ty::Addr, &[s1, zero]);
        let s2 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.inst(Op::Store, Ty::Unit, &[s2, x]);
        b.inst(Op::Store, Ty::Unit, &[e1, x]);
        // A slot alive past the next one's making through the jump that passes it on.
        let s3 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.inst(Op::Store, Ty::Unit, &[s3, x]);
        let s4 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.inst(Op::Store, Ty::Unit, &[s4, x]);
        let join = b.block(&[Ty::Addr]);
        b.jump(join, vec![s3]);
        b.switch(join);
        let p = b.params(join)[0];
        let y = b.inst(Op::Load, u8t, &[p]);
        // Two slots, the first dead before the second is made.
        let s5 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.inst(Op::Store, Ty::Unit, &[s5, y]);
        let s6 = b.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]);
        b.inst(Op::Store, Ty::Unit, &[s6, y]);
        b.ret(Some(y));
        let mut f = b.finish();
        f.frame = 0;
        let l = layout(&f).unwrap();
        let at = |s: NodeId| l.offsets[s.0 as usize].unwrap();
        assert!(apart((at(s1), 16), (at(s2), 16)), "{:?}", l.offsets);
        assert!(apart((at(s3), 16), (at(s4), 16)), "{:?}", l.offsets);
        assert_eq!(at(s5), at(s6), "{:?}", l.offsets);
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

    /// Slots alive together lie apart, each at the alignment its size implies and none rounded up:
    /// `a`'s literal and `a`, three bytes each, then `b`'s literal and `b`, five each, take 16
    /// bytes in all, where rounding each slot up to 16 took 48. A slot one run makes after
    /// another starts at an offset of its own while the frame holds them.
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
        assert_eq!(slots, [(0, 3), (3, 3), (6, 5), (11, 5)]);
        assert_eq!(layout.bytes, 16);
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
        assert_eq!(slots, [(0, 4), (0, 4), (4, 4), (8, 4), (12, 4)]);
        assert_eq!(layout.bytes, 16);
    }

    /// A slot takes the alignment its size implies, the largest power of two up to 16 that
    /// divides it, which is at least the alignment of what it holds: five `u8` variables alive
    /// together take five bytes, under the frame of 73 item 25 counts, where rounding each to 16
    /// took 80 (the review of pull request 183).
    #[test]
    fn test_a_slot_takes_the_alignment_its_size_implies() {
        for (bytes, align) in [
            (0, 1),
            (1, 1),
            (2, 2),
            (3, 1),
            (4, 4),
            (6, 2),
            (8, 8),
            (12, 4),
            (16, 16),
            (24, 8),
            (32, 16),
            (100, 4),
            (4096, 16),
        ] {
            assert_eq!(align_of(bytes), align, "{bytes}");
        }
        assert_eq!(align_up(13, 8), 16);
        assert_eq!(align_up(16, 8), 16);
        let source = "module m\npub fn five(x: u8) -> u8 {\n  var a: u8 = x\n  var b: u8 = x\n  var c: u8 = x\n  var d: u8 = x\n  var e: u8 = x\n  a = a / 2\n  b = b / 2\n  c = c / 2\n  d = d / 2\n  e = e / 2\n  a + b + c + d + e\n}\n";
        let (layout, slots) = layout_of(source, "five");
        assert_eq!(slots, [(0, 1), (1, 1), (2, 1), (3, 1), (4, 1)]);
        assert_eq!(layout.bytes, 5);
    }

    /// A slot of no bytes, as an empty array's, takes one, so that a function whose slots all hold
    /// nothing still has an area their addresses lie in: one whose only slot held nothing had none,
    /// and native code refused it, as "a slot with no place in its function's slots" (generated
    /// program of seed 126, 2026-11-22).
    #[test]
    fn test_a_slot_of_no_bytes_takes_one() {
        let source = "module m\npub fn f(c: bool) -> u8 {\n  _ = [0_u8; 0]\n  3\n}\n";
        let (layout, slots) = layout_of(source, "f");
        assert_eq!(slots.len(), 1, "{slots:?}");
        assert!(slots.iter().all(|(_, bytes)| *bytes == 0), "{slots:?}");
        assert!(layout.bytes >= 1, "{}", layout.bytes);
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        for target in [
            crate::Target::X86_64Linux,
            crate::Target::Aarch64Linux,
            crate::Target::X86_64Windows,
        ] {
            assert!(
                crate::native::frames_and_slots(&lowered, target).is_ok(),
                "{target:?}"
            );
        }
    }

    /// Slots past what one stack slot holds, 4 GiB, are a limit of the compiler, reported as one,
    /// never an overflow: a 4 GiB array and the binding it is copied into, alive together at the
    /// copy, in a branch that never runs, as the review of pull request 183 wrote it.
    #[test]
    fn test_slots_past_four_gib_are_a_limit_not_an_overflow() {
        let source = "module m\npub fn f(x: u8) -> u8 {\n  if x == 7 {\n    val a: [4294967295]u8 = [x; 4294967295]\n    _ = a\n  }\n  x\n}\n";
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        let f = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == "f")
            .unwrap();
        let refused = layout(f).err().unwrap_or_default();
        assert!(
            refused.contains("past what a stack slot holds"),
            "{refused}"
        );
    }
}

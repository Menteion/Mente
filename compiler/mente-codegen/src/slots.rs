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
    // The slots alive together at each point: those of the addresses alive there, of the
    // instruction's operands, and of the value it defines, which takes its slot's bytes where it
    // is made even if nothing reads it.
    let mut together: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); sizes.len()];
    let mut meet = |slots: &BTreeSet<usize>| {
        for &a in slots {
            together[a].extend(slots.iter().copied().filter(|b| *b != a));
        }
    };
    let slots_of = |vs: &mut dyn Iterator<Item = NodeId>| -> BTreeSet<usize> {
        vs.flat_map(|v| roots[v.0 as usize].iter().copied())
            .collect()
    };
    for block in &f.blocks {
        let mut live = live_after(block, &live_in);
        meet(&slots_of(&mut live.iter().copied()));
        for &v in block.insts.iter().rev() {
            let at = slots_of(
                &mut live
                    .iter()
                    .copied()
                    .chain([v])
                    .chain(f.args(v).iter().copied()),
            );
            meet(&at);
            live.remove(&v);
            live.extend(f.args(v).iter().copied().filter(|a| rooted(a)));
        }
        meet(&slots_of(
            &mut live.iter().copied().chain(block.params.iter().copied()),
        ));
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
    let mut reaches = |from: (usize, usize), to: (usize, usize)| -> bool {
        if from.0 == to.0 && from.1 < to.1 {
            return true;
        }
        let blocks = reached[from.0].get_or_insert_with(|| {
            // The blocks one jump or more from `from`'s block.
            let mut seen = vec![false; f.blocks.len()];
            let mut stack: Vec<usize> = targets(&f.blocks[from.0].term)
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
        blocks[to.0]
    };
    // Each slot at the lowest offset clear of every slot placed before it that is alive with it
    // in one run.
    let mut placed: Vec<Option<u32>> = vec![None; sizes.len()];
    let mut bytes = 0;
    for k in 0..sizes.len() {
        let mut taken: Vec<(u32, u32)> = Vec::new();
        for &o in &together[k] {
            let Some(at) = placed[o] else {
                continue;
            };
            if reaches(where_made[o], where_made[k]) || reaches(where_made[k], where_made[o]) {
                taken.push((at, at + sizes[o]));
            }
        }
        taken.sort_unstable();
        let mut at = 0;
        for (start, end) in taken {
            if at + sizes[k] <= start {
                break;
            }
            at = at.max(end);
        }
        placed[k] = Some(at);
        offsets[makers[k].0 as usize] = Some(at);
        bytes = bytes.max(at + sizes[k]);
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

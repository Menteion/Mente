//! `mente-mir` — Mente IR, written from scratch in Rust. Governed by `spec/mir-and-codegen.md`.
//!
//! What exists today:
//! - One flat, straight-line region per function: parallel `Vec`s (structure of arrays) indexed by `NodeId(u32)`.
//!   There is no control flow, no nested region and no RVSDG yet.
//! - A `ValueLattice` interval per node, supplied by whoever builds the region; nothing infers it yet.
//! - A `@branchless` check that rejects `BranchCondJump` nodes.
//! - An in-memory interpreter and an opcode-flipping mutation gate.
//! - [`cfg`]: the MIR of M1's checkpoint (c), an SSA control-flow graph with block parameters,
//!   stored as structure of arrays, with its verifier and interpreter.

pub mod cfg;

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(pub u32);

/// Physical memory geometry of a container (`spec/morphic-layouts.md`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MorphicLayout {
    /// Standard Array-of-Structs (`List<T>`)
    AoS,
    /// Contiguous Structure-of-Arrays (`SoA<T>`)
    SoA,
    /// SIMD/Cache-Line Tiled Array-of-Structs-of-Arrays (`Tiled<T, lanes: N>`)
    Tiled { lanes: u32 },
}

impl MorphicLayout {
    /// Computes the byte offset of element `elem_idx` and field `field_idx` (given field sizes and total element count).
    pub fn byte_offset(
        &self,
        elem_idx: usize,
        field_idx: usize,
        field_sizes: &[usize],
        total_elems: usize,
    ) -> usize {
        let field_offset_in_struct: usize = field_sizes[..field_idx].iter().sum();
        let struct_size: usize = field_sizes.iter().sum();
        match *self {
            MorphicLayout::AoS => elem_idx * struct_size + field_offset_in_struct,
            MorphicLayout::SoA => {
                let prior_arrays_bytes: usize = field_sizes[..field_idx]
                    .iter()
                    .map(|s| s * total_elems)
                    .sum();
                prior_arrays_bytes + elem_idx * field_sizes[field_idx]
            }
            MorphicLayout::Tiled { lanes } => {
                let l = lanes as usize;
                let tile_idx = elem_idx / l;
                let lane_idx = elem_idx % l;
                let tile_bytes = struct_size * l;
                let prior_fields_in_tile = field_offset_in_struct * l;
                tile_idx * tile_bytes + prior_fields_in_tile + lane_idx * field_sizes[field_idx]
            }
        }
    }
}

/// Interval and alignment facts attached to a MIR value. Today the builder of the region writes them;
/// `spec/refinement-and-fixed-point.md` specifies inferring them.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ValueLattice {
    pub min_val: i128,
    pub max_val: i128,
    pub align_bytes: u32,
}

impl ValueLattice {
    pub const fn exact(v: i128) -> Self {
        Self {
            min_val: v,
            max_val: v,
            align_bytes: 8,
        }
    }

    pub const fn bounded(min_val: i128, max_val: i128) -> Self {
        Self {
            min_val,
            max_val,
            align_bytes: 8,
        }
    }

    pub const fn proves_in_bounds(&self, slice_len: usize) -> bool {
        self.min_val >= 0 && self.max_val < (slice_len as i128)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MirType {
    I64,
    Q16_16,
    Bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MirOp {
    Param(u16),
    Const(i64),
    Add,
    Sub,
    AddSatQ16,
    SubSatQ16,
    Shr,
    CmpGe,
    CmpLt,
    CmpEq,
    /// Hardware branchless conditional select (`cmov` on x86-64 / `csel` on ARM64).
    SelectBranchless,
    /// Traditional conditional jump (FORBIDDEN inside `@branchless` functions!).
    BranchCondJump,
}

/// Structure-of-arrays MIR region.
///
/// `#[repr(align(64))]` aligns only this header (name, flags and the `Vec` handles). The node
/// arrays live in heap `Vec`s with their element types' alignment, not on 64-byte lines.
#[repr(align(64))]
#[derive(Clone, Debug)]
pub struct MirRegionSoA {
    pub name: String,
    pub is_branchless_guaranteed: bool,
    pub opcodes: Vec<MirOp>,
    pub types: Vec<MirType>,
    pub inputs: Vec<[NodeId; 3]>,
    pub lattices: Vec<ValueLattice>,
    pub lines: Vec<u32>,
    pub ret_node: Option<NodeId>,
}

impl MirRegionSoA {
    pub fn new(name: impl Into<String>, is_branchless_guaranteed: bool) -> Self {
        Self {
            name: name.into(),
            is_branchless_guaranteed,
            opcodes: Vec::with_capacity(64),
            types: Vec::with_capacity(64),
            inputs: Vec::with_capacity(64),
            lattices: Vec::with_capacity(64),
            lines: Vec::with_capacity(64),
            ret_node: None,
        }
    }

    pub fn push_node(
        &mut self,
        op: MirOp,
        ty: MirType,
        in_nodes: [NodeId; 3],
        lattice: ValueLattice,
        line: u32,
    ) -> NodeId {
        let id = NodeId(self.opcodes.len() as u32);
        self.opcodes.push(op);
        self.types.push(ty);
        self.inputs.push(in_nodes);
        self.lattices.push(lattice);
        self.lines.push(line);
        id
    }

    /// Rejects a `@branchless` region that contains a `BranchCondJump` node (`E0401`).
    pub fn verify_branchless(&self) -> Result<(), MirViolation> {
        if !self.is_branchless_guaranteed {
            return Ok(());
        }
        for (idx, op) in self.opcodes.iter().enumerate() {
            if *op == MirOp::BranchCondJump {
                return Err(MirViolation {
                    code: "E0401",
                    node: NodeId(idx as u32),
                    line: self.lines[idx],
                    message: format!(
                        "function `{}` is marked `@branchless`, but MIR node #{} is a conditional jump; use `SelectBranchless`",
                        self.name, idx
                    ),
                });
            }
        }
        Ok(())
    }

    /// Evaluates the MIR region in memory (zero disk I/O, sub-microsecond execution).
    pub fn eval_in_memory(&self, args: &[i64]) -> i64 {
        let n = self.opcodes.len();
        let mut vals = vec![0i64; n];
        for i in 0..n {
            let inp = self.inputs[i];
            let a = vals.get(inp[0].0 as usize).copied().unwrap_or(0);
            let b = vals.get(inp[1].0 as usize).copied().unwrap_or(0);
            let c = vals.get(inp[2].0 as usize).copied().unwrap_or(0);
            vals[i] = match self.opcodes[i] {
                MirOp::Param(idx) => args.get(idx as usize).copied().unwrap_or(0),
                MirOp::Const(v) => v,
                MirOp::Add => a.wrapping_add(b),
                MirOp::Sub => a.wrapping_sub(b),
                MirOp::AddSatQ16 => (a as i32).saturating_add(b as i32) as i64,
                MirOp::SubSatQ16 => (a as i32).saturating_sub(b as i32) as i64,
                MirOp::Shr => a >> (b.clamp(0, 63)),
                MirOp::CmpGe => {
                    if a >= b {
                        1
                    } else {
                        0
                    }
                }
                MirOp::CmpLt => {
                    if a < b {
                        1
                    } else {
                        0
                    }
                }
                MirOp::CmpEq => {
                    if a == b {
                        1
                    } else {
                        0
                    }
                }
                MirOp::SelectBranchless => {
                    if a != 0 {
                        b
                    } else {
                        c
                    }
                }
                MirOp::BranchCondJump => {
                    if a != 0 {
                        b
                    } else {
                        c
                    }
                }
            };
        }
        self.ret_node.map(|r| vals[r.0 as usize]).unwrap_or(0)
    }
}

/// A rule a MIR region breaks. MIR has no source spans until `.mn` code is lowered into it
/// (M1), so it names the node and the source line; `mn` reports it with its code
/// (`spec/diagnostics.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirViolation {
    pub code: &'static str,
    pub node: NodeId,
    pub line: u32,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct MutationReport {
    pub total_mutants: usize,
    pub killed_mutants: usize,
    pub kill_score_pct: f64,
}

/// Runs in-memory MIR Mutation Proving against a test suite `(inputs, expected_output)`.
///
/// Flips operators (`Add` <-> `Sub`, `CmpGe` <-> `CmpLt`, `SelectBranchless` swap) directly in the SoA
/// array in memory—completing in microseconds without touching the disk!
pub fn run_in_memory_mutation_gate(
    region: &MirRegionSoA,
    test_vectors: &[(&[i64], i64)],
) -> MutationReport {
    let mut total_mutants = 0usize;
    let mut killed_mutants = 0usize;

    for idx in 0..region.opcodes.len() {
        let original_op = region.opcodes[idx];
        let mutated_op = match original_op {
            MirOp::Add => Some(MirOp::Sub),
            MirOp::Sub => Some(MirOp::Add),
            MirOp::AddSatQ16 => Some(MirOp::SubSatQ16),
            MirOp::SubSatQ16 => Some(MirOp::AddSatQ16),
            MirOp::CmpGe => Some(MirOp::CmpLt),
            MirOp::CmpLt => Some(MirOp::CmpGe),
            _ => None,
        };

        if let Some(m_op) = mutated_op {
            total_mutants += 1;
            let mut mutant = region.clone();
            mutant.opcodes[idx] = m_op;
            let killed = test_vectors
                .iter()
                .any(|(args, expected)| mutant.eval_in_memory(args) != *expected);
            if killed {
                killed_mutants += 1;
            }
        }

        if original_op == MirOp::SelectBranchless {
            total_mutants += 1;
            let mut mutant = region.clone();
            let [cond, t_val, f_val] = mutant.inputs[idx];
            mutant.inputs[idx] = [cond, f_val, t_val]; // Swap true/false lanes
            let killed = test_vectors
                .iter()
                .any(|(args, expected)| mutant.eval_in_memory(args) != *expected);
            if killed {
                killed_mutants += 1;
            }
        }
    }

    let kill_score_pct = if total_mutants > 0 {
        (killed_mutants as f64 / total_mutants as f64) * 100.0
    } else {
        100.0
    };

    MutationReport {
        total_mutants,
        killed_mutants,
        kill_score_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branchless_integrate_cell_mir_and_mutation_prover() {
        // Build the MIR for `integrate_cell` from examples/02_mechanical_sympathy_simd.mn:
        // leaked = potential - (potential >> leak_shift)
        // next_v = leaked + drive
        // fired  = next_v >= threshold
        // next_potential = select(fired, resting, next_v)
        let mut r = MirRegionSoA::new("integrate_cell", true);
        let z = NodeId(0);
        let potential = r.push_node(
            MirOp::Param(0),
            MirType::Q16_16,
            [z, z, z],
            ValueLattice::bounded(0, 655360),
            1,
        );
        let drive = r.push_node(
            MirOp::Param(1),
            MirType::Q16_16,
            [z, z, z],
            ValueLattice::bounded(0, 655360),
            1,
        );
        let leak_shift = r.push_node(
            MirOp::Param(2),
            MirType::I64,
            [z, z, z],
            ValueLattice::bounded(1, 8),
            1,
        );
        let threshold = r.push_node(
            MirOp::Param(3),
            MirType::Q16_16,
            [z, z, z],
            ValueLattice::exact(65536),
            1,
        );
        let resting = r.push_node(
            MirOp::Param(4),
            MirType::Q16_16,
            [z, z, z],
            ValueLattice::exact(0),
            1,
        );

        let shifted = r.push_node(
            MirOp::Shr,
            MirType::Q16_16,
            [potential, leak_shift, z],
            ValueLattice::bounded(0, 327680),
            2,
        );
        let leaked = r.push_node(
            MirOp::SubSatQ16,
            MirType::Q16_16,
            [potential, shifted, z],
            ValueLattice::bounded(0, 655360),
            2,
        );
        let next_v = r.push_node(
            MirOp::AddSatQ16,
            MirType::Q16_16,
            [leaked, drive, z],
            ValueLattice::bounded(0, 655360),
            3,
        );
        let fired = r.push_node(
            MirOp::CmpGe,
            MirType::Bool,
            [next_v, threshold, z],
            ValueLattice::bounded(0, 1),
            4,
        );
        let out_v = r.push_node(
            MirOp::SelectBranchless,
            MirType::Q16_16,
            [fired, resting, next_v],
            ValueLattice::bounded(0, 65536),
            5,
        );
        r.ret_node = Some(out_v);

        // 1. Verify @branchless passes
        assert!(r.verify_branchless().is_ok());

        // 2. Evaluate in memory: potential=32768 (0.5_q16), drive=49152 (0.75_q16), shift=2, threshold=65536 (1.0_q16), resting=0
        // leaked = 32768 - 8192 = 24576; next_v = 24576 + 49152 = 73728 >= 65536 -> fired! returns resting (0)
        let v1 = r.eval_in_memory(&[32768, 49152, 2, 65536, 0]);
        assert_eq!(v1, 0);

        // Sub-threshold case: drive=16384 (0.25_q16) -> next_v = 24576 + 16384 = 40960 < 65536 -> returns 40960
        let v2 = r.eval_in_memory(&[32768, 16384, 2, 65536, 0]);
        assert_eq!(v2, 40960);

        // 3. Run in-memory mutation prover: 100% of mutants must be killed!
        let mut_report = run_in_memory_mutation_gate(
            &r,
            &[
                (&[32768, 49152, 2, 65536, 0], 0),
                (&[32768, 16384, 2, 65536, 0], 40960),
            ],
        );
        assert_eq!(
            (mut_report.total_mutants, mut_report.killed_mutants),
            (4, 4)
        );
        assert_eq!(mut_report.kill_score_pct, 100.0);
    }

    /// `out = select(a >= b, a - b, a + b)`: one mutant for each of `CmpGe`, `Sub` and `Add`,
    /// and one for the swapped select arms.
    fn select_region() -> MirRegionSoA {
        let mut r = MirRegionSoA::new("diff_or_sum", true);
        let z = NodeId(0);
        let any = ValueLattice::bounded(-100, 100);
        let a = r.push_node(MirOp::Param(0), MirType::I64, [z, z, z], any, 1);
        let b = r.push_node(MirOp::Param(1), MirType::I64, [z, z, z], any, 1);
        let ge = r.push_node(
            MirOp::CmpGe,
            MirType::Bool,
            [a, b, z],
            ValueLattice::bounded(0, 1),
            2,
        );
        let diff = r.push_node(MirOp::Sub, MirType::I64, [a, b, z], any, 2);
        let sum = r.push_node(MirOp::Add, MirType::I64, [a, b, z], any, 2);
        r.ret_node = Some(r.push_node(
            MirOp::SelectBranchless,
            MirType::I64,
            [ge, diff, sum],
            any,
            3,
        ));
        r
    }

    #[test]
    fn test_mutation_gate_counts_survivors() {
        let r = select_region();
        assert_eq!(r.eval_in_memory(&[7, 3]), 4);
        assert_eq!(r.eval_in_memory(&[3, 7]), 10);

        // Both arms of the select checked: every mutant is killed.
        let strong = run_in_memory_mutation_gate(&r, &[(&[7, 3], 4), (&[3, 7], 10)]);
        assert_eq!((strong.total_mutants, strong.killed_mutants), (4, 4));

        // Only the `a >= b` arm checked: flipping `a + b` to `a - b` in the unchecked arm survives.
        let weak = run_in_memory_mutation_gate(&r, &[(&[7, 3], 4)]);
        assert_eq!((weak.total_mutants, weak.killed_mutants), (4, 3));
        assert_eq!(weak.kill_score_pct, 75.0);

        // A region with nothing to flip reports no mutants rather than a failure.
        let mut constant = MirRegionSoA::new("constant", false);
        let z = NodeId(0);
        constant.ret_node = Some(constant.push_node(
            MirOp::Const(5),
            MirType::I64,
            [z, z, z],
            ValueLattice::exact(5),
            1,
        ));
        let none = run_in_memory_mutation_gate(&constant, &[(&[], 5)]);
        assert_eq!(
            (none.total_mutants, none.killed_mutants, none.kill_score_pct),
            (0, 0, 100.0)
        );
    }

    /// `out = op(a, b)` for one binary opcode.
    fn binary_region(op: MirOp) -> MirRegionSoA {
        let mut r = MirRegionSoA::new("binary", false);
        let z = NodeId(0);
        let any = ValueLattice::bounded(-100, 100);
        let a = r.push_node(MirOp::Param(0), MirType::I64, [z, z, z], any, 1);
        let b = r.push_node(MirOp::Param(1), MirType::I64, [z, z, z], any, 1);
        r.ret_node =
            Some(r.push_node(op, MirType::Bool, [a, b, z], ValueLattice::bounded(0, 1), 2));
        r
    }

    #[test]
    fn test_interpreter_compares_and_selects() {
        for (op, below, equal, above) in [
            (MirOp::CmpLt, 1, 0, 0),
            (MirOp::CmpGe, 0, 1, 1),
            (MirOp::CmpEq, 0, 1, 0),
        ] {
            let r = binary_region(op);
            assert_eq!(
                [
                    r.eval_in_memory(&[2, 3]),
                    r.eval_in_memory(&[3, 3]),
                    r.eval_in_memory(&[4, 3])
                ],
                [below, equal, above],
                "{:?}",
                op
            );
        }
        // A conditional jump evaluates like a select: the true arm when the condition is non-zero.
        let mut r = MirRegionSoA::new("jump", false);
        let z = NodeId(0);
        let any = ValueLattice::bounded(-100, 100);
        let c = r.push_node(
            MirOp::Param(0),
            MirType::Bool,
            [z, z, z],
            ValueLattice::bounded(0, 1),
            1,
        );
        let t = r.push_node(MirOp::Const(10), MirType::I64, [z, z, z], any, 1);
        let f = r.push_node(MirOp::Const(20), MirType::I64, [z, z, z], any, 1);
        r.ret_node = Some(r.push_node(MirOp::BranchCondJump, MirType::I64, [c, t, f], any, 2));
        assert_eq!((r.eval_in_memory(&[1]), r.eval_in_memory(&[0])), (10, 20));
    }

    #[test]
    fn test_mutation_gate_flips_each_comparison() {
        // `a < b` becomes `a >= b`, and the reverse; one vector on each side kills it.
        for (op, vectors) in [
            (MirOp::CmpLt, [(&[2i64, 3][..], 1i64), (&[4, 3][..], 0)]),
            (MirOp::CmpGe, [(&[2i64, 3][..], 0i64), (&[4, 3][..], 1)]),
        ] {
            let rep = run_in_memory_mutation_gate(&binary_region(op), &vectors);
            assert_eq!((rep.total_mutants, rep.killed_mutants), (1, 1), "{:?}", op);
        }
        // `CmpEq` has no flip yet.
        let rep = run_in_memory_mutation_gate(&binary_region(MirOp::CmpEq), &[(&[3, 3], 1)]);
        assert_eq!(rep.total_mutants, 0);
    }

    #[test]
    fn test_nodes_are_rows_addressed_by_index() {
        let mut r = MirRegionSoA::new("rows", false);
        let z = NodeId(0);
        let a = r.push_node(
            MirOp::Param(0),
            MirType::I64,
            [z, z, z],
            ValueLattice::bounded(0, 9),
            10,
        );
        let c = r.push_node(
            MirOp::Const(5),
            MirType::I64,
            [z, z, z],
            ValueLattice::exact(5),
            11,
        );
        let sum = r.push_node(
            MirOp::Add,
            MirType::I64,
            [a, c, z],
            ValueLattice::bounded(5, 14),
            12,
        );
        assert_eq!((a, c, sum), (NodeId(0), NodeId(1), NodeId(2)));
        // Each node is one row across the parallel arrays.
        assert_eq!(
            r.opcodes,
            vec![MirOp::Param(0), MirOp::Const(5), MirOp::Add]
        );
        assert_eq!(r.types.len(), 3);
        assert_eq!(r.inputs[2], [a, c, z]);
        assert_eq!(r.lattices[2], ValueLattice::bounded(5, 14));
        assert_eq!(r.lines, vec![10, 11, 12]);
        r.ret_node = Some(sum);
        assert_eq!(r.eval_in_memory(&[4]), 9);
    }

    #[test]
    fn test_lattice_proves_index_in_bounds() {
        assert!(ValueLattice::bounded(0, 9).proves_in_bounds(10));
        assert!(!ValueLattice::bounded(0, 10).proves_in_bounds(10));
        assert!(!ValueLattice::bounded(-1, 5).proves_in_bounds(10));
        assert!(ValueLattice::exact(0).proves_in_bounds(1));
        assert!(!ValueLattice::exact(0).proves_in_bounds(0));
    }

    #[test]
    fn test_saturating_q16_ops_clamp() {
        let mut r = MirRegionSoA::new("sat", false);
        let z = NodeId(0);
        let any = ValueLattice::bounded(i32::MIN as i128, i32::MAX as i128);
        let a = r.push_node(MirOp::Param(0), MirType::Q16_16, [z, z, z], any, 1);
        let b = r.push_node(MirOp::Param(1), MirType::Q16_16, [z, z, z], any, 1);
        let add = r.push_node(MirOp::AddSatQ16, MirType::Q16_16, [a, b, z], any, 2);
        let sub = r.push_node(MirOp::SubSatQ16, MirType::Q16_16, [a, b, z], any, 2);
        let max = i32::MAX as i64;
        let min = i32::MIN as i64;

        r.ret_node = Some(add);
        assert_eq!(r.eval_in_memory(&[65536, 32768]), 98304); // 1.0 + 0.5
        assert_eq!(r.eval_in_memory(&[max - 1, 65536]), max);
        r.ret_node = Some(sub);
        assert_eq!(r.eval_in_memory(&[65536, 32768]), 32768); // 1.0 - 0.5
        assert_eq!(r.eval_in_memory(&[min + 1, 65536]), min);
    }

    #[test]
    fn test_branchless_region_rejects_conditional_jump() {
        let mut r = select_region();
        assert!(r.verify_branchless().is_ok());
        r.push_node(
            MirOp::BranchCondJump,
            MirType::I64,
            [NodeId(2), NodeId(3), NodeId(4)],
            ValueLattice::bounded(-100, 100),
            7,
        );
        let err = r.verify_branchless().unwrap_err();
        assert_eq!((err.code, err.node, err.line), ("E0401", NodeId(6), 7));
        assert!(err.message.contains("node #6"), "{}", err.message);
        // A region not marked @branchless may jump.
        r.is_branchless_guaranteed = false;
        assert!(r.verify_branchless().is_ok());
    }

    #[test]
    fn test_morphic_layout_offsets() {
        let fields = [12usize, 12usize, 4usize]; // e.g., pos:12B, vel:12B, mass:4B (total 28B)
        let tiled = MorphicLayout::Tiled { lanes: 16 };
        // In a 16-lane AoSoA tile, field 0 (pos) occupies [0 .. 192), field 1 (vel) starts at 192
        assert_eq!(tiled.byte_offset(0, 1, &fields, 1024), 192);
        assert_eq!(tiled.byte_offset(1, 1, &fields, 1024), 192 + 12);
        // Element 17 is lane 1 of the second tile, which starts at 16 * 28 bytes.
        assert_eq!(tiled.byte_offset(17, 2, &fields, 1024), 448 + 384 + 4);

        // AoS: whole elements one after another.
        assert_eq!(
            MorphicLayout::AoS.byte_offset(3, 1, &fields, 1024),
            3 * 28 + 12
        );
        // SoA: one array per field, each total_elems long.
        assert_eq!(
            MorphicLayout::SoA.byte_offset(3, 1, &fields, 1024),
            12 * 1024 + 3 * 12
        );
        assert_eq!(
            MorphicLayout::SoA.byte_offset(5, 2, &fields, 1024),
            24 * 1024 + 5 * 4
        );
    }
}

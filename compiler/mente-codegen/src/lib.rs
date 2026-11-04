//! `mente-codegen` — backend placeholders. Governed by `spec/error-handling-abi.md` and `spec/mir-and-codegen.md`.
//!
//! Nothing here emits machine code. `CarryFlagAbiLowering` only names the registers the specified
//! error ABI would use, and `emit_spirv_binary` writes a SPIR-V-shaped word stream that is not a valid module.

use mente_mir::{MirOp, MirRegionSoA};

/// The registers and flag the specified Carry-Flag error ABI assigns per target (`spec/error-handling-abi.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarryFlagAbiLowering {
    /// Primary return register carrying `T` on the Happy Path (`RAX` on x86-64, `X0` on ARM64).
    pub value_register: &'static str,
    /// Secondary error payload register used ONLY when Carry Flag (`CF=1`) is set (`RDX` on x86-64, `X1` on ARM64).
    pub error_register: &'static str,
    /// CPU status flag indicating error (`CF` on x86-64/ARM64).
    pub status_flag: &'static str,
    /// Zero extra stack or tag bytes added to `T` on the happy path.
    pub happy_path_tag_overhead_bytes: usize,
}

impl CarryFlagAbiLowering {
    pub const fn x86_64() -> Self {
        Self {
            value_register: "RAX",
            error_register: "RDX",
            status_flag: "CF (RFLAGS.0)",
            happy_path_tag_overhead_bytes: 0,
        }
    }

    pub const fn aarch64() -> Self {
        Self {
            value_register: "X0",
            error_register: "X1",
            status_flag: "C (NZCV.29)",
            happy_path_tag_overhead_bytes: 0,
        }
    }
}

/// SPIR-V magic number (`0x07230203`).
pub const SPIRV_MAGIC: u32 = 0x0723_0203;
pub const SPIRV_VERSION_1_6: u32 = 0x0001_0600;
pub const MENTE_GENERATOR_MAGIC: u32 = 0x4D45_0001;

/// Writes a SPIR-V 1.6 header, `OpCapability Shader`, and one 4-word record per MIR node.
///
/// The output is not a valid SPIR-V module: it has no `OpMemoryModel`, `OpEntryPoint`, types or
/// function, and the per-node records do not follow each opcode's operand layout. `spirv-val` rejects it.
pub fn emit_spirv_binary(region: &MirRegionSoA) -> Vec<u32> {
    let bound = (region.opcodes.len() as u32) + 16;
    let mut words = vec![
        SPIRV_MAGIC,
        SPIRV_VERSION_1_6,
        MENTE_GENERATOR_MAGIC,
        bound,
        0, // Reserved instruction schema
    ];

    // OpCapability Shader (op=17, word_count=2, cap=1)
    words.push((2 << 16) | 17);
    words.push(1);

    // Encode each MIR node into deterministic SPIR-V word stream
    for (idx, op) in region.opcodes.iter().enumerate() {
        let opcode_id: u32 = match op {
            MirOp::Param(_) => 59,           // OpFunctionParameter
            MirOp::Const(_) => 43,           // OpConstant
            MirOp::Add | MirOp::AddSatQ16 => 128, // OpIAdd
            MirOp::Sub | MirOp::SubSatQ16 => 130, // OpISub
            MirOp::Shr => 196,               // OpShiftRightArithmetic
            MirOp::CmpGe => 173,             // OpSGreaterThanEqual
            MirOp::CmpLt => 177,             // OpSLessThan
            MirOp::CmpEq => 170,             // OpIEqual
            MirOp::SelectBranchless => 169,  // OpSelect (Branchless hardware select!)
            MirOp::BranchCondJump => 250,    // OpBranchConditional
        };
        let result_id = (idx as u32) + 10;
        words.push((4 << 16) | opcode_id);
        words.push(result_id);
        words.push(region.inputs[idx][0].0);
        words.push(region.inputs[idx][1].0);
    }

    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_mir::{MirType, NodeId, ValueLattice};

    #[test]
    fn test_carry_flag_abi_zero_overhead() {
        let abi = CarryFlagAbiLowering::x86_64();
        assert_eq!(abi.happy_path_tag_overhead_bytes, 0);
        assert_eq!(abi.value_register, "RAX");
    }

    #[test]
    fn test_direct_spirv_emission_from_mir() {
        let mut r = MirRegionSoA::new("step_particles", true);
        let z = NodeId(0);
        let a = r.push_node(MirOp::Param(0), MirType::I64, [z, z, z], ValueLattice::exact(10), 1);
        let b = r.push_node(MirOp::Param(1), MirType::I64, [z, z, z], ValueLattice::exact(20), 1);
        let sum = r.push_node(MirOp::Add, MirType::I64, [a, b, z], ValueLattice::exact(30), 2);
        r.ret_node = Some(sum);

        let spv = emit_spirv_binary(&r);
        assert_eq!(spv[0], SPIRV_MAGIC);
        assert_eq!(spv[1], SPIRV_VERSION_1_6);
    }
}

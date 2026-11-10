//! Native code from MIR (`spec/mir-and-codegen.md` item 5, [`native-runtime`]).
//!
//! Each MIR function becomes a Cranelift function, one MIR block one Cranelift block, so that
//! the native program computes what the MIR interpreter does. What Cranelift and the C library
//! do not give, this module generates in Cranelift IR as well:
//!
//! - a runtime: writing bytes and integers, the latter in decimal by 32-bit limbs, and dividing
//!   128-bit integers, which Cranelift lowers only up to 64 bits;
//! - for every trap site, code that writes the reference interpreter's message to standard
//!   error and exits with status 1;
//! - `main`, which sets up one call's arguments, calls its entry and writes what `mn run`
//!   writes.
//!
//! The only functions it imports are the C library's `write` (`_write` on Windows), `exit`
//! and `memcpy`. A 128-bit integer crosses a function boundary as two 64-bit halves, low then
//! high, and a 128-bit result is stored through a pointer the caller passes first, so that no
//! calling convention's treatment of `i128`, nor its count of result registers, matters.
//!
//! [`native-runtime`]: ../../docs/design-review.md#native-runtime

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    self, types, AbiParam, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind, TrapCode, Type,
    Value,
};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId as ClFunc, Linkage, Module};
use mente_mir::cfg::{self, ArithOp, BitOp, CmpOp, IntTy, Num, Op, Term};
use mente_mir::NodeId;

use crate::lower::{Entry, Lowered, Shape};
use crate::{CodegenError, ObjectWriter, SourcePos, Target};

/// One call the program makes: an entry of the lowered module, by its index, and its
/// arguments.
#[derive(Clone, Debug)]
pub struct Call {
    pub entry: usize,
    pub args: Vec<Arg>,
}

/// An argument from outside the program.
#[derive(Clone, Debug)]
pub enum Arg {
    Int(Num),
    Bool(bool),
    Array(Vec<Arg>),
}

fn err(message: impl Into<String>) -> CodegenError {
    CodegenError::new(message.into())
}

/// The Cranelift type of a MIR type, an address being the target's pointer.
fn cl_type(t: cfg::Ty, ptr: Type) -> Type {
    match t {
        cfg::Ty::Int(i) => int_type(i),
        cfg::Ty::Bool => types::I8,
        cfg::Ty::Addr => ptr,
        cfg::Ty::Unit => types::I8,
    }
}

fn int_type(i: IntTy) -> Type {
    match i.bits {
        8 => types::I8,
        16 => types::I16,
        32 => types::I32,
        64 => types::I64,
        _ => types::I128,
    }
}

/// The parameters or results a value of `t` takes at a function boundary: an `i128` as two
/// `i64`.
fn abi(t: Type) -> Vec<AbiParam> {
    if t == types::I128 {
        vec![AbiParam::new(types::I64), AbiParam::new(types::I64)]
    } else {
        vec![AbiParam::new(t)]
    }
}

/// The functions the program imports and the runtime it carries, with its strings.
struct Runtime {
    write: ClFunc,
    exit: ClFunc,
    print: ClFunc,
    udivmod: ClFunc,
    strings: HashMap<String, DataId>,
}

/// The trap a call of `entry` from outside the program raises on `args` before anything else,
/// if an argument does not fit its parameter's type: the reference interpreter narrows every
/// argument into its parameter's type first, in order and element by element, at the function's
/// name, and reports the first value that does not fit. The entry takes each argument in its
/// parameter's own type, which cannot hold such a value, so whoever calls the entry narrows
/// every argument this way first; the entry then checks the refinements, as the interpreter
/// does once every argument is narrowed.
pub fn refusal(entry: &Entry, args: &[Arg]) -> Option<String> {
    fn first(shape: &Shape, arg: &Arg) -> Option<(Num, &'static str)> {
        match (shape, arg) {
            (Shape::Int(t), Arg::Int(n)) => {
                (!crate::lower::mir(*t).holds(*n)).then_some((*n, t.name))
            }
            (Shape::Array(_, elem), Arg::Array(items)) => items.iter().find_map(|a| first(elem, a)),
            _ => None,
        }
    }
    entry
        .params
        .iter()
        .zip(args)
        .find_map(|((_, shape), arg)| first(shape, arg))
        .map(|(n, t)| {
            format!(
                "trap[narrowing] at {}:{}: {} does not fit in `{}`",
                entry.at.line, entry.at.col, n, t
            )
        })
}

/// Writes an object with every function of `lowered`, the runtime, and a `main` that makes the
/// call `argv[1]` numbers among `calls`, the first when there is none; `prefix` starts every
/// trap's message, as `mn run` writes `path: `.
pub fn program(
    lowered: &Lowered,
    calls: &[Call],
    prefix: &str,
    writer: &mut ObjectWriter,
) -> Result<(), CodegenError> {
    let ptr = writer.module().target_config().pointer_type();
    let conv = writer.module().target_config().default_call_conv;
    let windows = matches!(writer.target(), Target::X86_64Windows);
    // The imports.
    let mut sig = ir::Signature::new(conv);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    sig.params
        .push(AbiParam::new(if windows { types::I32 } else { ptr }));
    sig.returns
        .push(AbiParam::new(if windows { types::I32 } else { ptr }));
    let module = writer.module_mut();
    let write = module
        .declare_function(
            if windows { "_write" } else { "write" },
            Linkage::Import,
            &sig,
        )
        .map_err(|e| err(e.to_string()))?;
    let mut sig = ir::Signature::new(conv);
    sig.params.push(AbiParam::new(types::I32));
    let exit = module
        .declare_function("exit", Linkage::Import, &sig)
        .map_err(|e| err(e.to_string()))?;
    // The runtime.
    let mut sig = ir::Signature::new(conv);
    for t in [types::I32, types::I8, types::I64, types::I64] {
        sig.params.push(AbiParam::new(t));
    }
    let print = module
        .declare_function("mente_rt_print", Linkage::Local, &sig)
        .map_err(|e| err(e.to_string()))?;
    let mut sig = ir::Signature::new(conv);
    for _ in 0..4 {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.params.push(AbiParam::new(ptr));
    let udivmod = module
        .declare_function("mente_rt_udivmod", Linkage::Local, &sig)
        .map_err(|e| err(e.to_string()))?;
    let mut rt = Runtime {
        write,
        exit,
        print,
        udivmod,
        strings: HashMap::new(),
    };
    // The MIR functions.
    let mut ids = Vec::new();
    for (i, f) in lowered.module.functions.iter().enumerate() {
        let sig = signature(f, conv, ptr);
        let name = format!("mente_f{}_{}", i, sanitize(&f.name));
        ids.push(
            writer
                .module_mut()
                .declare_function(&name, Linkage::Local, &sig)
                .map_err(|e| err(e.to_string()))?,
        );
    }
    let mut fctx = FunctionBuilderContext::new();
    define_print(writer, &rt, &mut fctx, ptr)?;
    define_udivmod(writer, &rt, &mut fctx, ptr)?;
    for (i, f) in lowered.module.functions.iter().enumerate() {
        let decl =
            f.locs
                .iter()
                .find(|l| l.line != 0)
                .map_or(SourcePos { line: 1, column: 1 }, |l| SourcePos {
                    line: l.line,
                    column: l.col,
                });
        let mut ctx = writer.module().make_context();
        ctx.func.signature = signature(f, conv, ptr);
        Translate::function(writer, &mut rt, &ids, f, &mut ctx, &mut fctx, ptr, prefix)?;
        writer.define_function_as(ids[i], &mut ctx, decl, &f.name)?;
    }
    define_main(
        writer,
        &mut rt,
        &ids,
        (lowered, calls),
        &mut fctx,
        (ptr, conv),
        prefix,
    )?;
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn signature(
    f: &cfg::Function,
    conv: cranelift_codegen::isa::CallConv,
    ptr: Type,
) -> ir::Signature {
    let mut sig = ir::Signature::new(conv);
    let wide = returns_wide(f, ptr);
    if wide {
        sig.params.push(AbiParam::new(ptr));
    }
    for p in &f.params {
        sig.params.extend(abi(cl_type(*p, ptr)));
    }
    if let Some(r) = f.result.filter(|_| !wide) {
        sig.returns.push(AbiParam::new(cl_type(r, ptr)));
    }
    sig
}

/// Whether `f` returns a 128-bit integer, which goes through a pointer.
fn returns_wide(f: &cfg::Function, ptr: Type) -> bool {
    f.result.is_some_and(|r| cl_type(r, ptr) == types::I128)
}

/// The data object holding `text`, declared once.
fn string(writer: &mut ObjectWriter, rt: &mut Runtime, text: &str) -> Result<DataId, CodegenError> {
    if let Some(id) = rt.strings.get(text) {
        return Ok(*id);
    }
    let name = format!("mente_str{}", rt.strings.len());
    let module = writer.module_mut();
    let id = module
        .declare_data(&name, Linkage::Local, false, false)
        .map_err(|e| err(e.to_string()))?;
    let mut data = DataDescription::new();
    data.define(text.as_bytes().to_vec().into_boxed_slice());
    module
        .define_data(id, &data)
        .map_err(|e| err(e.to_string()))?;
    rt.strings.insert(text.to_string(), id);
    Ok(id)
}

/// Flags for an access known aligned and not trapping.
fn trusted(_b: &mut FunctionBuilder) -> MemFlagsData {
    MemFlagsData::trusted()
}

/// Helpers over a function under construction.
struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
}

impl Emit<'_, '_> {
    fn iconst(&mut self, t: Type, n: i64) -> Value {
        self.b.ins().iconst(t, n)
    }

    /// The constant `n` of type `t`, `n` holding in it.
    fn int(&mut self, t: Type, n: Num, i: IntTy) -> Value {
        let bits = n.to_bits(i);
        if t == types::I128 {
            let lo = self.b.ins().iconst(types::I64, bits as u64 as i64);
            let hi = self.b.ins().iconst(types::I64, (bits >> 64) as u64 as i64);
            self.b.ins().iconcat(lo, hi)
        } else {
            self.b.ins().iconst(t, bits as u64 as i64)
        }
    }

    /// A value of `t` widened to `i128`, by its sign when `signed`.
    fn to128(&mut self, v: Value, t: Type, signed: bool) -> Value {
        if t == types::I128 {
            v
        } else if signed {
            self.b.ins().sextend(types::I128, v)
        } else {
            self.b.ins().uextend(types::I128, v)
        }
    }

    /// `v` of integer type `i` as a sign and a magnitude below 2^128.
    fn sign_mag(&mut self, v: Value, i: IntTy) -> (Value, Value) {
        let t = int_type(i);
        if !i.signed {
            let zero = self.iconst(types::I8, 0);
            return (zero, self.to128(v, t, false));
        }
        let zero = self.int(t, Num::ZERO, i);
        let neg = self.b.ins().icmp(IntCC::SignedLessThan, v, zero);
        let flipped = self.b.ins().isub(zero, v);
        let mag = self.b.ins().select(neg, flipped, v);
        (neg, self.to128(mag, t, false))
    }

    /// Splits a value of `t` for a function boundary.
    fn split(&mut self, v: Value, t: Type) -> Vec<Value> {
        if t == types::I128 {
            let (lo, hi) = self.b.ins().isplit(v);
            vec![lo, hi]
        } else {
            vec![v]
        }
    }

    fn data_addr(&mut self, module: &mut cranelift_object::ObjectModule, id: DataId) -> Value {
        let gv = module.declare_data_in_func(id, self.b.func);
        self.b.ins().symbol_value(self.ptr, gv)
    }
}

/// Writes `text` to file descriptor `fd`.
fn emit_text(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    e: &mut Emit,
    fd: i64,
    text: &str,
) -> Result<(), CodegenError> {
    if text.is_empty() {
        return Ok(());
    }
    let id = string(writer, rt, text)?;
    let addr = e.data_addr(writer.module_mut(), id);
    let windows = matches!(writer.target(), Target::X86_64Windows);
    let len_ty = if windows { types::I32 } else { e.ptr };
    let fd = e.iconst(types::I32, fd);
    let len = e.iconst(len_ty, text.len() as i64);
    let f = writer.module_mut().declare_func_in_func(rt.write, e.b.func);
    e.b.ins().call(f, &[fd, addr, len]);
    Ok(())
}

/// Writes the integer `v` of type `i` to `fd`, in decimal.
fn emit_num(writer: &mut ObjectWriter, rt: &Runtime, e: &mut Emit, fd: i64, v: Value, i: IntTy) {
    let (neg, mag) = e.sign_mag(v, i);
    emit_sign_mag(writer, rt, e, fd, neg, mag);
}

fn emit_sign_mag(
    writer: &mut ObjectWriter,
    rt: &Runtime,
    e: &mut Emit,
    fd: i64,
    neg: Value,
    mag: Value,
) {
    let (lo, hi) = e.b.ins().isplit(mag);
    let fd = e.iconst(types::I32, fd);
    let f = writer.module_mut().declare_func_in_func(rt.print, e.b.func);
    e.b.ins().call(f, &[fd, neg, hi, lo]);
}

/// Exits with `status`; what follows is unreachable.
fn emit_exit(writer: &mut ObjectWriter, rt: &Runtime, e: &mut Emit, status: i64) {
    let s = e.iconst(types::I32, status);
    let f = writer.module_mut().declare_func_in_func(rt.exit, e.b.func);
    e.b.ins().call(f, &[s]);
    // `exit` never returns; the trap only ends the block.
    e.b.ins().trap(TrapCode::STACK_OVERFLOW);
}

/// `mente_rt_print(fd, neg, hi, lo)`: writes the integer whose sign is `neg` and whose
/// magnitude is `hi · 2^64 + lo`, in decimal, dividing four 32-bit limbs by ten until all are
/// zero.
fn define_print(
    writer: &mut ObjectWriter,
    rt: &Runtime,
    fctx: &mut FunctionBuilderContext,
    ptr: Type,
) -> Result<(), CodegenError> {
    let mut ctx = writer.module().make_context();
    let frontend = writer.module().isa().frontend_config();
    let conv = writer.module().target_config().default_call_conv;
    ctx.func.signature = ir::Signature::new(conv);
    for t in [types::I32, types::I8, types::I64, types::I64] {
        ctx.func.signature.params.push(AbiParam::new(t));
    }
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let [fd, neg, hi, lo] = b.block_params(entry)[..] else {
            unreachable!("four parameters")
        };
        let slot =
            b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 48, 4));
        let mask = b.ins().iconst(types::I64, 0xffff_ffff);
        let l3 = b.ins().ushr_imm_u(hi, 32);
        let l2 = b.ins().band(hi, mask);
        let l1 = b.ins().ushr_imm_u(lo, 32);
        let l0 = b.ins().band(lo, mask);
        let digit = b.create_block();
        for _ in 0..5 {
            b.append_block_param(digit, types::I64);
        }
        let start = b.ins().iconst(types::I64, 48);
        b.ins().jump(
            digit,
            &[l3.into(), l2.into(), l1.into(), l0.into(), start.into()],
        );
        b.switch_to_block(digit);
        let p = b.block_params(digit).to_vec();
        let ten = b.ins().iconst(types::I64, 10);
        let mut rem = b.ins().iconst(types::I64, 0);
        let mut limbs = Vec::new();
        for &l in &p[..4] {
            let shifted = b.ins().ishl_imm_u(rem, 32);
            let cur = b.ins().bor(shifted, l);
            limbs.push(b.ins().udiv(cur, ten));
            rem = b.ins().urem(cur, ten);
        }
        let one = b.ins().iconst(types::I64, 1);
        let pos = b.ins().isub(p[4], one);
        let base = b.ins().stack_addr(ptr, slot, 0);
        let at = b.ins().iadd(base, pos);
        let zero_char = b.ins().iconst(types::I64, i64::from(b'0'));
        let ch = b.ins().iadd(rem, zero_char);
        let ch = b.ins().ireduce(types::I8, ch);
        let fl = trusted(&mut b);
        b.ins().store(fl, ch, at, 0);
        let any01 = b.ins().bor(limbs[0], limbs[1]);
        let any23 = b.ins().bor(limbs[2], limbs[3]);
        let any = b.ins().bor(any01, any23);
        let done = b.create_block();
        b.append_block_param(done, types::I64);
        b.ins().brif(
            any,
            digit,
            &[
                limbs[0].into(),
                limbs[1].into(),
                limbs[2].into(),
                limbs[3].into(),
                pos.into(),
            ],
            done,
            &[pos.into()],
        );
        b.switch_to_block(done);
        let pos = b.block_params(done)[0];
        let minus = b.create_block();
        let out = b.create_block();
        b.append_block_param(out, types::I64);
        b.ins().brif(neg, minus, &[], out, &[pos.into()]);
        b.switch_to_block(minus);
        let pos2 = b.ins().isub(pos, one);
        let at = b.ins().iadd(base, pos2);
        let dash = b.ins().iconst(types::I8, i64::from(b'-'));
        let fl = trusted(&mut b);
        b.ins().store(fl, dash, at, 0);
        b.ins().jump(out, &[pos2.into()]);
        b.switch_to_block(out);
        let pos = b.block_params(out)[0];
        let at = b.ins().iadd(base, pos);
        let end = b.ins().iconst(types::I64, 48);
        let len = b.ins().isub(end, pos);
        let windows = matches!(writer.target(), Target::X86_64Windows);
        let len = if windows {
            b.ins().ireduce(types::I32, len)
        } else {
            len
        };
        let f = writer.module_mut().declare_func_in_func(rt.write, b.func);
        b.ins().call(f, &[fd, at, len]);
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(frontend);
    }
    writer.define_function(rt.print, &mut ctx, SourcePos { line: 1, column: 1 })
}

/// `mente_rt_udivmod(a_lo, a_hi, b_lo, b_hi, out)`: unsigned 128-bit division, storing `q_lo,
/// q_hi, r_lo, r_hi` at `out`; `b` is never zero. When both fit in 64 bits, one 64-bit division;
/// when the divisor fits in 32 bits, three, a 32-bit digit of the quotient's low half at a time
/// after its high half; otherwise shifting and subtracting, one bit a round.
fn define_udivmod(
    writer: &mut ObjectWriter,
    rt: &Runtime,
    fctx: &mut FunctionBuilderContext,
    ptr: Type,
) -> Result<(), CodegenError> {
    let mut ctx = writer.module().make_context();
    let frontend = writer.module().isa().frontend_config();
    let conv = writer.module().target_config().default_call_conv;
    ctx.func.signature = ir::Signature::new(conv);
    for _ in 0..4 {
        ctx.func.signature.params.push(AbiParam::new(types::I64));
    }
    ctx.func.signature.params.push(AbiParam::new(ptr));
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let [alo, ahi, blo, bhi, out] = b.block_params(entry)[..] else {
            unreachable!("five parameters")
        };
        // The stores of a quotient and remainder given as 64-bit halves.
        let store = |b: &mut FunctionBuilder, halves: [Value; 4]| {
            for (k, v) in halves.into_iter().enumerate() {
                let fl = trusted(b);
                b.ins().store(fl, v, out, 8 * k as i32);
            }
            b.ins().return_(&[]);
        };
        let z = b.ins().iconst(types::I64, 0);
        // Both in 64 bits: one division.
        let narrow = b.create_block();
        let wide = b.create_block();
        let both = b.ins().bor(ahi, bhi);
        let small = b.ins().icmp_imm_u(IntCC::Equal, both, 0);
        b.ins().brif(small, narrow, &[], wide, &[]);
        b.switch_to_block(narrow);
        let q = b.ins().udiv(alo, blo);
        let r = b.ins().urem(alo, blo);
        store(&mut b, [q, z, r, z]);
        // A divisor in 32 bits: the high half, then two 32-bit digits, each step's dividend below
        // the divisor times 2^32, so that its quotient fits a digit.
        b.switch_to_block(wide);
        let digits = b.create_block();
        let serial = b.create_block();
        let fits = b
            .ins()
            .icmp_imm_u(IntCC::UnsignedLessThanOrEqual, blo, 0xFFFF_FFFF);
        let short = b.ins().icmp_imm_u(IntCC::Equal, bhi, 0);
        let short = b.ins().band(short, fits);
        b.ins().brif(short, digits, &[], serial, &[]);
        b.switch_to_block(digits);
        let qhi = b.ins().udiv(ahi, blo);
        let r1 = b.ins().urem(ahi, blo);
        let up = b.ins().ishl_imm_u(r1, 32);
        let top = b.ins().ushr_imm_u(alo, 32);
        let x = b.ins().bor(up, top);
        let q1 = b.ins().udiv(x, blo);
        let r2 = b.ins().urem(x, blo);
        let up = b.ins().ishl_imm_u(r2, 32);
        let bottom = b.ins().band_imm_u(alo, 0xFFFF_FFFF);
        let y = b.ins().bor(up, bottom);
        let q0 = b.ins().udiv(y, blo);
        let r = b.ins().urem(y, blo);
        let q1 = b.ins().ishl_imm_u(q1, 32);
        let qlo = b.ins().bor(q1, q0);
        store(&mut b, [qlo, qhi, r, z]);
        // Otherwise, a bit a round.
        b.switch_to_block(serial);
        let a = b.ins().iconcat(alo, ahi);
        let d = b.ins().iconcat(blo, bhi);
        let round = b.create_block();
        b.append_block_param(round, types::I64);
        b.append_block_param(round, types::I128);
        b.append_block_param(round, types::I128);
        let start = b.ins().iconst(types::I64, 127);
        let z64 = b.ins().iconst(types::I64, 0);
        let zero = b.ins().iconcat(z64, z64);
        b.ins()
            .jump(round, &[start.into(), zero.into(), zero.into()]);
        b.switch_to_block(round);
        let [i, q, r] = b.block_params(round)[..] else {
            unreachable!("three parameters")
        };
        let bit = b.ins().ushr(a, i);
        let one64 = b.ins().iconst(types::I64, 1);
        let one = b.ins().uextend(types::I128, one64);
        let bit = b.ins().band(bit, one);
        let r2 = b.ins().ishl_imm_u(r, 1);
        let r2 = b.ins().bor(r2, bit);
        let ge = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, r2, d);
        let sub = b.ins().isub(r2, d);
        let r3 = b.ins().select(ge, sub, r2);
        let mark = b.ins().ishl(one, i);
        let q2 = b.ins().bor(q, mark);
        let q3 = b.ins().select(ge, q2, q);
        let done = b.create_block();
        let next = b.create_block();
        let last = b.ins().icmp_imm_u(IntCC::Equal, i, 0);
        b.ins().brif(last, done, &[], next, &[]);
        b.switch_to_block(next);
        let i2 = b.ins().isub(i, one64);
        b.ins().jump(round, &[i2.into(), q3.into(), r3.into()]);
        b.switch_to_block(done);
        let (qlo, qhi) = b.ins().isplit(q3);
        let (rlo, rhi) = b.ins().isplit(r3);
        store(&mut b, [qlo, qhi, rlo, rhi]);
        b.seal_all_blocks();
        b.finalize(frontend);
    }
    writer.define_function(rt.udivmod, &mut ctx, SourcePos { line: 1, column: 1 })
}

/// The translation of one MIR function.
struct Translate;

impl Translate {
    #[allow(clippy::too_many_arguments)]
    fn function(
        writer: &mut ObjectWriter,
        rt: &mut Runtime,
        ids: &[ClFunc],
        f: &cfg::Function,
        ctx: &mut Context,
        fctx: &mut FunctionBuilderContext,
        ptr: Type,
        prefix: &str,
    ) -> Result<(), CodegenError> {
        let frontend = writer.module().isa().frontend_config();
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let blocks: Vec<ir::Block> = f.blocks.iter().map(|_| b.create_block()).collect();
        let mut vals: Vec<Option<Value>> = vec![None; f.ops.len()];
        let ty = |v: NodeId| cl_type(f.ty(v), ptr);
        // Block parameters; the entry's come split at the boundary.
        for (bi, block) in f.blocks.iter().enumerate() {
            if bi == 0 {
                continue;
            }
            for &p in &block.params {
                vals[p.0 as usize] = Some(b.append_block_param(blocks[bi], ty(p)));
            }
        }
        b.switch_to_block(blocks[0]);
        let ret_ptr = returns_wide(f, ptr).then(|| b.append_block_param(blocks[0], ptr));
        // Every parameter first, then the halves of each 128-bit one joined.
        let raw: Vec<Vec<Value>> = f.blocks[0]
            .params
            .iter()
            .map(|&p| match ty(p) {
                t if t == types::I128 => vec![
                    b.append_block_param(blocks[0], types::I64),
                    b.append_block_param(blocks[0], types::I64),
                ],
                t => vec![b.append_block_param(blocks[0], t)],
            })
            .collect();
        let entry_params: Vec<Value> = raw
            .into_iter()
            .map(|halves| match halves[..] {
                [lo, hi] => b.ins().iconcat(lo, hi),
                _ => halves[0],
            })
            .collect();
        for (p, v) in f.blocks[0].params.iter().zip(entry_params) {
            vals[p.0 as usize] = Some(v);
        }
        for (bi, block) in f.blocks.iter().enumerate() {
            if bi > 0 {
                b.switch_to_block(blocks[bi]);
            }
            for &v in &block.insts {
                let loc = f.locs[v.0 as usize];
                if loc.line != 0 {
                    let sl = writer.source_loc(SourcePos {
                        line: loc.line,
                        column: loc.col,
                    })?;
                    b.set_srcloc(sl);
                }
                let mut e = Emit { b: &mut b, ptr };
                let value = Self::inst(writer, rt, ids, f, &mut e, &vals, v, prefix)?;
                vals[v.0 as usize] = value;
            }
            let get = |v: &NodeId| {
                vals[v.0 as usize].ok_or_else(|| err("a value used before it is defined"))
            };
            let args = |vs: &[NodeId]| -> Result<Vec<ir::BlockArg>, CodegenError> {
                vs.iter().map(|v| get(v).map(Into::into)).collect()
            };
            match &block.term {
                Term::Jump(t, a) => {
                    b.ins().jump(blocks[t.0 as usize], &args(a)?);
                }
                Term::Branch {
                    cond,
                    then,
                    otherwise,
                } => {
                    let (c, yes, no) = (get(cond)?, args(&then.1)?, args(&otherwise.1)?);
                    b.ins().brif(
                        c,
                        blocks[then.0 .0 as usize],
                        &yes,
                        blocks[otherwise.0 .0 as usize],
                        &no,
                    );
                }
                Term::Return(r) => {
                    let mut out = Vec::new();
                    match (r, ret_ptr) {
                        (Some(r), Some(at)) => {
                            let fl = trusted(&mut b);
                            let v = get(r)?;
                            b.ins().store(fl, v, at, 0);
                        }
                        (Some(r), None) => out.push(get(r)?),
                        _ => {}
                    }
                    b.ins().return_(&out);
                }
                Term::Trap(site) => {
                    let mut e = Emit { b: &mut b, ptr };
                    trap_report(writer, rt, &mut e, f, *site, &[], prefix)?;
                }
            }
        }
        b.seal_all_blocks();
        b.finalize(frontend);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn inst(
        writer: &mut ObjectWriter,
        rt: &mut Runtime,
        ids: &[ClFunc],
        f: &cfg::Function,
        e: &mut Emit,
        vals: &[Option<Value>],
        v: NodeId,
        prefix: &str,
    ) -> Result<Option<Value>, CodegenError> {
        let ptr = e.ptr;
        let args: Vec<Value> = f
            .args(v)
            .iter()
            .map(|a| vals[a.0 as usize].ok_or_else(|| err("a value used before it is defined")))
            .collect::<Result<_, _>>()?;
        let arg_tys: Vec<cfg::Ty> = f.args(v).iter().map(|a| f.ty(*a)).collect();
        let rty = f.ty(v);
        let t = cl_type(rty, ptr);
        let int = |x: cfg::Ty| match x {
            cfg::Ty::Int(i) => Ok(i),
            _ => Err(err("an integer operation on a value that is not one")),
        };
        Ok(Some(match f.op(v) {
            Op::Param => return Err(err("a parameter among instructions")),
            Op::Int(bits) => {
                let i = int(rty)?;
                e.int(t, Num::from_bits(*bits, i), i)
            }
            Op::Bool(x) => e.iconst(types::I8, i64::from(*x)),
            Op::Arith { op, trap: None } => {
                let i = int(rty)?;
                match op {
                    ArithOp::Add => e.b.ins().iadd(args[0], args[1]),
                    ArithOp::Sub => e.b.ins().isub(args[0], args[1]),
                    ArithOp::Mul => e.b.ins().imul(args[0], args[1]),
                    ArithOp::Div | ArithOp::Rem if t == types::I128 => {
                        let divisor = constant(f, f.args(v)[1]).map(|n| n.mag);
                        let (q, r) = divide128(writer, rt, e, (args[0], args[1]), divisor, i);
                        if *op == ArithOp::Div {
                            q
                        } else {
                            r
                        }
                    }
                    ArithOp::Div if i.signed => e.b.ins().sdiv(args[0], args[1]),
                    ArithOp::Div => e.b.ins().udiv(args[0], args[1]),
                    ArithOp::Rem if i.signed => e.b.ins().srem(args[0], args[1]),
                    ArithOp::Rem => e.b.ins().urem(args[0], args[1]),
                }
            }
            Op::Arith {
                op,
                trap: Some(site),
            } => {
                let i = int(rty)?;
                // Each operand in its own type, which may differ from the checked one.
                let (ta, tb) = (int(f.ty(f.args(v)[0]))?, int(f.ty(f.args(v)[1]))?);
                let (sa, ma) = e.sign_mag(args[0], ta);
                let (sb, mb) = e.sign_mag(args[1], tb);
                let (neg, mag, beyond) = match op {
                    ArithOp::Add | ArithOp::Sub => {
                        let sb = if *op == ArithOp::Sub {
                            let one = e.iconst(types::I8, 1);
                            e.b.ins().bxor(sb, one)
                        } else {
                            sb
                        };
                        sum_sign_mag(e, (sa, ma), (sb, mb))
                    }
                    ArithOp::Mul => {
                        let neg = e.b.ins().bxor(sa, sb);
                        let (mag, beyond) = mul128(e, ma, mb);
                        (neg, mag, beyond)
                    }
                    ArithOp::Div | ArithOp::Rem => {
                        let divisor = constant(f, f.args(v)[1]).map(|n| n.mag);
                        let (q, r) = divide_mag(writer, rt, e, (ma, mb), divisor);
                        let no = e.iconst(types::I8, 0);
                        if *op == ArithOp::Div {
                            (e.b.ins().bxor(sa, sb), q, no)
                        } else {
                            (sa, r, no)
                        }
                    }
                };
                checked_result(writer, rt, e, f, *site, (neg, mag, beyond), i, prefix)?
            }
            Op::Neg { trap: None } => e.b.ins().ineg(args[0]),
            Op::Neg { trap: Some(site) } => {
                let i = int(rty)?;
                let (s, m) = e.sign_mag(args[0], i);
                let one = e.iconst(types::I8, 1);
                let flipped = e.b.ins().bxor(s, one);
                let no = e.iconst(types::I8, 0);
                checked_result(writer, rt, e, f, *site, (flipped, m, no), i, prefix)?
            }
            Op::Bits(op) => match op {
                BitOp::And => e.b.ins().band(args[0], args[1]),
                BitOp::Or => e.b.ins().bor(args[0], args[1]),
                BitOp::Xor => e.b.ins().bxor(args[0], args[1]),
            },
            Op::Not => {
                if rty == cfg::Ty::Bool {
                    let one = e.iconst(types::I8, 1);
                    e.b.ins().bxor(args[0], one)
                } else {
                    e.b.ins().bnot(args[0])
                }
            }
            Op::Shl { trap: site } => {
                let i = int(rty)?;
                let from = int(arg_tys[0])?;
                let (s, m) = e.sign_mag(args[0], from);
                let k = args[1];
                let k128 = e.to128(k, types::I64, false);
                // Beyond 2^128 when the magnitude has fewer leading zeros than the shift.
                let lz = e.b.ins().clz(m);
                let beyond = e.b.ins().icmp(IntCC::UnsignedGreaterThan, k128, lz);
                let shifted = e.b.ins().ishl(m, k);
                checked_result(writer, rt, e, f, *site, (s, shifted, beyond), i, prefix)?
            }
            Op::Shr => {
                let i = int(rty)?;
                if i.signed {
                    e.b.ins().sshr(args[0], args[1])
                } else {
                    e.b.ins().ushr(args[0], args[1])
                }
            }
            Op::Cmp(op) => {
                let signed = matches!(arg_tys[0], cfg::Ty::Int(IntTy { signed: true, .. }));
                let cc = match (op, signed) {
                    (CmpOp::Eq, _) => IntCC::Equal,
                    (CmpOp::Ne, _) => IntCC::NotEqual,
                    (CmpOp::Lt, true) => IntCC::SignedLessThan,
                    (CmpOp::Le, true) => IntCC::SignedLessThanOrEqual,
                    (CmpOp::Gt, true) => IntCC::SignedGreaterThan,
                    (CmpOp::Ge, true) => IntCC::SignedGreaterThanOrEqual,
                    (CmpOp::Lt, false) => IntCC::UnsignedLessThan,
                    (CmpOp::Le, false) => IntCC::UnsignedLessThanOrEqual,
                    (CmpOp::Gt, false) => IntCC::UnsignedGreaterThan,
                    (CmpOp::Ge, false) => IntCC::UnsignedGreaterThanOrEqual,
                };
                e.b.ins().icmp(cc, args[0], args[1])
            }
            Op::Cast => {
                let from = int(arg_tys[0])?;
                let to = int(rty)?;
                let (ft, tt) = (int_type(from), int_type(to));
                match tt.bits().cmp(&ft.bits()) {
                    std::cmp::Ordering::Equal => args[0],
                    std::cmp::Ordering::Less => e.b.ins().ireduce(tt, args[0]),
                    std::cmp::Ordering::Greater if from.signed => e.b.ins().sextend(tt, args[0]),
                    std::cmp::Ordering::Greater => e.b.ins().uextend(tt, args[0]),
                }
            }
            Op::Check { trap: site } => {
                let ok = e.b.create_block();
                let fail = e.b.create_block();
                e.b.ins().brif(args[0], ok, &[], fail, &[]);
                e.b.switch_to_block(fail);
                let shown: Vec<(Value, IntTy)> = args[1..]
                    .iter()
                    .zip(&arg_tys[1..])
                    .filter_map(|(v, t)| match t {
                        cfg::Ty::Int(i) => Some((*v, *i)),
                        _ => None,
                    })
                    .collect();
                let values: Vec<Shown> = shown.into_iter().map(|(v, i)| Shown::Int(v, i)).collect();
                trap_report(writer, rt, e, f, *site, &values, prefix)?;
                e.b.switch_to_block(ok);
                return Ok(None);
            }
            Op::Slot { bytes } => {
                let slot = e.b.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    (*bytes).max(1),
                    4,
                ));
                e.b.ins().stack_addr(ptr, slot, 0)
            }
            Op::Elem { stride } => {
                let s = e.iconst(ptr, i64::from(*stride));
                let off = e.b.ins().imul(args[1], s);
                e.b.ins().iadd(args[0], off)
            }
            Op::Load => {
                let fl = trusted(e.b);
                e.b.ins().load(t, fl, args[0], 0)
            }
            Op::Store => {
                let fl = trusted(e.b);
                e.b.ins().store(fl, args[1], args[0], 0);
                return Ok(None);
            }
            Op::Copy { bytes } => {
                let n = e.iconst(ptr, i64::from(*bytes));
                let config = writer.module().target_config();
                e.b.call_memcpy(config, args[0], args[1], n);
                return Ok(None);
            }
            Op::Call(c) => {
                let callee = &ids[c.0 as usize];
                let fref = writer.module_mut().declare_func_in_func(*callee, e.b.func);
                let mut flat = Vec::new();
                let out = (t == types::I128 && rty != cfg::Ty::Unit).then(|| slot(e, 16));
                flat.extend(out);
                for (a, at) in args.iter().zip(&arg_tys) {
                    flat.extend(e.split(*a, cl_type(*at, ptr)));
                }
                let call = e.b.ins().call(fref, &flat);
                let results = e.b.inst_results(call).to_vec();
                return Ok(match (rty, out, results.first()) {
                    (cfg::Ty::Unit, _, _) => None,
                    (_, Some(at), _) => {
                        let fl = trusted(e.b);
                        Some(e.b.ins().load(types::I128, fl, at, 0))
                    }
                    (_, None, Some(r)) => Some(*r),
                    _ => return Err(err("a call without its result")),
                });
            }
        }))
    }
}

/// What a trap's message shows between its pieces of text.
enum Shown {
    Int(Value, IntTy),
    /// A sign and a magnitude, or `the result` when it lies beyond 2^128.
    Exact(Value, Value, Value),
}

/// Writes trap site `site`'s message, with `values`, to standard error and exits with status 1.
fn trap_report(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    e: &mut Emit,
    f: &cfg::Function,
    site: cfg::TrapId,
    values: &[Shown],
    prefix: &str,
) -> Result<(), CodegenError> {
    let s = f
        .traps
        .get(site.0 as usize)
        .ok_or_else(|| err("a trap at a site that does not exist"))?;
    let head = format!(
        "{}trap[{}] at {}:{}: {}",
        prefix,
        s.kind,
        s.loc.line,
        s.loc.col,
        s.text.first().map_or("", String::as_str)
    );
    emit_text(writer, rt, e, 2, &head)?;
    for (v, piece) in values.iter().zip(&s.text[1..]) {
        match v {
            Shown::Int(x, i) => emit_num(writer, rt, e, 2, *x, *i),
            Shown::Exact(neg, mag, beyond) => {
                let known = e.b.create_block();
                let unknown = e.b.create_block();
                let after = e.b.create_block();
                e.b.ins().brif(*beyond, unknown, &[], known, &[]);
                e.b.switch_to_block(known);
                emit_sign_mag(writer, rt, e, 2, *neg, *mag);
                e.b.ins().jump(after, &[]);
                e.b.switch_to_block(unknown);
                emit_text(writer, rt, e, 2, "the result")?;
                e.b.ins().jump(after, &[]);
                e.b.switch_to_block(after);
            }
        }
        emit_text(writer, rt, e, 2, piece)?;
    }
    emit_text(writer, rt, e, 2, "\n")?;
    emit_exit(writer, rt, e, 1);
    Ok(())
}

/// The sum of two signed magnitudes: its sign, its magnitude, and whether it lies beyond 2^128.
fn sum_sign_mag(
    e: &mut Emit,
    (sa, ma): (Value, Value),
    (sb, mb): (Value, Value),
) -> (Value, Value, Value) {
    let same = e.b.ins().icmp(IntCC::Equal, sa, sb);
    let total = e.b.ins().iadd(ma, mb);
    let carry = e.b.ins().icmp(IntCC::UnsignedLessThan, total, ma);
    let a_bigger = e.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, ma, mb);
    let ab = e.b.ins().isub(ma, mb);
    let ba = e.b.ins().isub(mb, ma);
    let diff = e.b.ins().select(a_bigger, ab, ba);
    let diff_sign = e.b.ins().select(a_bigger, sa, sb);
    let mag = e.b.ins().select(same, total, diff);
    let sign = e.b.ins().select(same, sa, diff_sign);
    let no = e.iconst(types::I8, 0);
    let beyond = e.b.ins().select(same, carry, no);
    // Zero has no sign.
    let zero = e.int(types::I128, Num::ZERO, IntTy::new(128, false));
    let nonzero = e.b.ins().icmp(IntCC::NotEqual, mag, zero);
    let sign = e.b.ins().band(sign, nonzero);
    (sign, mag, beyond)
}

/// The product of two magnitudes, and whether it lies beyond 2^128.
fn mul128(e: &mut Emit, a: Value, b: Value) -> (Value, Value) {
    let (a0, a1) = e.b.ins().isplit(a);
    let (b0, b1) = e.b.ins().isplit(b);
    let z = e.iconst(types::I64, 0);
    let a1z = e.b.ins().icmp(IntCC::NotEqual, a1, z);
    let b1z = e.b.ins().icmp(IntCC::NotEqual, b1, z);
    let both = e.b.ins().band(a1z, b1z);
    // One of a1 and b1 is zero: the cross term is a1·b0 or a0·b1, which must fit 64 bits.
    let c1 = e.b.ins().imul(a1, b0);
    let c1h = e.b.ins().umulhi(a1, b0);
    let c2 = e.b.ins().imul(a0, b1);
    let c2h = e.b.ins().umulhi(a0, b1);
    let cross = e.b.ins().iadd(c1, c2);
    let high = e.b.ins().bor(c1h, c2h);
    let over = e.b.ins().icmp(IntCC::NotEqual, high, z);
    let low_lo = e.b.ins().imul(a0, b0);
    let low_hi = e.b.ins().umulhi(a0, b0);
    let hi = e.b.ins().iadd(low_hi, cross);
    let carry = e.b.ins().icmp(IntCC::UnsignedLessThan, hi, low_hi);
    let beyond = e.b.ins().bor(both, over);
    let beyond = e.b.ins().bor(beyond, carry);
    (e.b.ins().iconcat(low_lo, hi), beyond)
}

/// The value of MIR value `v` when it is a constant: an `int`, or a cast of one into a type
/// that holds it, which keeps its value.
fn constant(f: &cfg::Function, v: NodeId) -> Option<Num> {
    let cfg::Ty::Int(t) = f.ty(v) else {
        return None;
    };
    match f.op(v) {
        Op::Int(bits) => Some(Num::from_bits(*bits, t)),
        Op::Cast => constant(f, *f.args(v).first()?).filter(|n| t.holds(*n)),
        _ => None,
    }
}

/// The shift that divides by `divisor`, when it is a power of two.
fn power_of_two(divisor: Option<u128>) -> Option<u32> {
    divisor
        .filter(|d| d.is_power_of_two())
        .map(u128::trailing_zeros)
}

/// Quotient and remainder of two magnitudes: by a shift and a mask when the divisor is a
/// constant power of two, and otherwise through the runtime.
fn divide_mag(
    writer: &mut ObjectWriter,
    rt: &Runtime,
    e: &mut Emit,
    (a, b): (Value, Value),
    divisor: Option<u128>,
) -> (Value, Value) {
    if let Some(k) = power_of_two(divisor) {
        let unsigned = IntTy::new(128, false);
        let low = e.int(types::I128, Num::new(false, (1u128 << k) - 1), unsigned);
        let q = e.b.ins().ushr_imm_u(a, i64::from(k));
        return (q, e.b.ins().band(a, low));
    }
    let (alo, ahi) = e.b.ins().isplit(a);
    let (blo, bhi) = e.b.ins().isplit(b);
    let out = slot(e, 32);
    let f = writer
        .module_mut()
        .declare_func_in_func(rt.udivmod, e.b.func);
    e.b.ins().call(f, &[alo, ahi, blo, bhi, out]);
    let r: Vec<Value> = (0..4)
        .map(|k| {
            let fl = trusted(e.b);
            e.b.ins().load(types::I64, fl, out, 8 * k)
        })
        .collect();
    (e.b.ins().iconcat(r[0], r[1]), e.b.ins().iconcat(r[2], r[3]))
}

/// Exact 128-bit division: the quotient truncated toward zero and the remainder with the
/// dividend's sign; `divisor` is the divisor's magnitude when it is a constant.
fn divide128(
    writer: &mut ObjectWriter,
    rt: &Runtime,
    e: &mut Emit,
    (a, b): (Value, Value),
    divisor: Option<u128>,
    i: IntTy,
) -> (Value, Value) {
    let (sa, ma) = e.sign_mag(a, i);
    let (sb, mb) = e.sign_mag(b, i);
    let (q, r) = divide_mag(writer, rt, e, (ma, mb), divisor);
    let sq = e.b.ins().bxor(sa, sb);
    let nq = e.b.ins().ineg(q);
    let nr = e.b.ins().ineg(r);
    (e.b.ins().select(sq, nq, q), e.b.ins().select(sa, nr, r))
}

/// The value of a sign and a magnitude in type `i`, or a trap at `site` reporting the exact
/// value when it does not fit.
#[allow(clippy::too_many_arguments)]
fn checked_result(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    e: &mut Emit,
    f: &cfg::Function,
    site: cfg::TrapId,
    (neg, mag, beyond): (Value, Value, Value),
    i: IntTy,
    prefix: &str,
) -> Result<Value, CodegenError> {
    let max = e.int(types::I128, i.max(), IntTy::new(128, false));
    let min_mag = e.int(
        types::I128,
        Num::new(false, i.min().mag),
        IntTy::new(128, false),
    );
    let pos_ok = e.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, mag, max);
    let neg_ok = e.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, mag, min_mag);
    let fits = e.b.ins().select(neg, neg_ok, pos_ok);
    let one = e.iconst(types::I8, 1);
    let inside = e.b.ins().bxor(beyond, one);
    let ok = e.b.ins().band(fits, inside);
    let good = e.b.create_block();
    let bad = e.b.create_block();
    e.b.ins().brif(ok, good, &[], bad, &[]);
    e.b.switch_to_block(bad);
    trap_report(
        writer,
        rt,
        e,
        f,
        site,
        &[Shown::Exact(neg, mag, beyond)],
        prefix,
    )?;
    e.b.switch_to_block(good);
    let flipped = e.b.ins().ineg(mag);
    let v = e.b.ins().select(neg, flipped, mag);
    let t = int_type(i);
    Ok(if t == types::I128 {
        v
    } else {
        e.b.ins().ireduce(t, v)
    })
}

/// `main`: sets up `call`'s arguments in memory where they are arrays or `mut`, calls its
/// entry, and writes the result and what each `mut` argument leaves, as `mn run` does.
fn define_main(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    ids: &[ClFunc],
    (lowered, calls): (&Lowered, &[Call]),
    fctx: &mut FunctionBuilderContext,
    (ptr, conv): (Type, cranelift_codegen::isa::CallConv),
    prefix: &str,
) -> Result<(), CodegenError> {
    let mut sig = ir::Signature::new(conv);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::I32));
    let id = writer
        .module_mut()
        .declare_function("main", Linkage::Export, &sig)
        .map_err(|e| err(e.to_string()))?;
    let mut ctx = writer.module().make_context();
    let frontend = writer.module().isa().frontend_config();
    ctx.func.signature = sig;
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let [argc, argv] = b.block_params(block)[..] else {
            unreachable!("two parameters")
        };
        // The call's number: argv[1] in decimal, or 0 without it.
        let parse = b.create_block();
        let dispatch = b.create_block();
        b.append_block_param(dispatch, types::I64);
        let zero64 = b.ins().iconst(types::I64, 0);
        let has = b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, argc, 2);
        b.ins().brif(has, parse, &[], dispatch, &[zero64.into()]);
        b.switch_to_block(parse);
        let fl = trusted(&mut b);
        let text = b
            .ins()
            .load(ptr, fl, argv, i32::try_from(ptr.bytes()).unwrap_or(8));
        let digit = b.create_block();
        b.append_block_param(digit, ptr);
        b.append_block_param(digit, types::I64);
        b.ins().jump(digit, &[text.into(), zero64.into()]);
        b.switch_to_block(digit);
        let [at, n] = b.block_params(digit)[..] else {
            unreachable!("two parameters")
        };
        let fl = trusted(&mut b);
        let c = b.ins().uload8(types::I64, fl, at, 0);
        let more = b.create_block();
        let end = b.ins().icmp_imm_u(IntCC::Equal, c, 0);
        b.ins().brif(end, dispatch, &[n.into()], more, &[]);
        b.switch_to_block(more);
        let ten = b.ins().imul_imm_s(n, 10);
        let d = b.ins().iadd_imm_s(c, -i64::from(b'0'));
        let n2 = b.ins().iadd(ten, d);
        let next = b.ins().iadd_imm_s(at, 1);
        b.ins().jump(digit, &[next.into(), n2.into()]);
        b.switch_to_block(dispatch);
        let index = b.block_params(dispatch)[0];
        let mut tests = b.create_block();
        b.ins().jump(tests, &[]);
        let mut shared = Shared {
            slot: b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 4)),
            used: 0,
            most: 1,
        };
        for (k, call) in calls.iter().enumerate() {
            b.switch_to_block(tests);
            let here = b.create_block();
            tests = b.create_block();
            let is = b.ins().icmp_imm_u(IntCC::Equal, index, k as i64);
            b.ins().brif(is, here, &[], tests, &[]);
            b.switch_to_block(here);
            shared.used = 0;
            make_call(
                writer,
                rt,
                ids,
                (lowered, call),
                (&mut b, &mut shared),
                ptr,
                prefix,
            )?;
        }
        b.func.sized_stack_slots[shared.slot].size = shared.most;
        b.switch_to_block(tests);
        let none = b.ins().iconst(types::I32, 2);
        b.ins().return_(&[none]);
        b.seal_all_blocks();
        b.finalize(frontend);
    }
    writer.define_function(id, &mut ctx, SourcePos { line: 1, column: 1 })
}

/// The code of one call in `main`: its arguments, the call of its entry, and what `mn run`
/// writes, then a return of 0; or, for an argument that does not fit its parameter's type, the
/// reference interpreter's trap ([`refusal`]).
fn make_call(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    ids: &[ClFunc],
    (lowered, call): (&Lowered, &Call),
    (b, shared): (&mut FunctionBuilder, &mut Shared),
    ptr: Type,
    prefix: &str,
) -> Result<(), CodegenError> {
    let entry = lowered
        .entries
        .get(call.entry)
        .ok_or_else(|| err("a call of an entry that does not exist"))?;
    let mir = &lowered.module.functions[entry.func.0 as usize];
    {
        let mut e = Emit { b, ptr };
        if let Some(trap) = refusal(entry, &call.args) {
            emit_text(writer, rt, &mut e, 2, &format!("{}{}\n", prefix, trap))?;
            emit_exit(writer, rt, &mut e, 1);
            return Ok(());
        }
        let mut params = mir.params.iter();
        let mut flat = Vec::new();
        let wide = if returns_wide(mir, ptr) {
            Some(shared.take(&mut e, 16)?)
        } else {
            None
        };
        flat.extend(wide);
        let sret = if matches!(entry.result, Shape::Array(..)) {
            params.next();
            let slot = shared.take(&mut e, entry.result.size())?;
            flat.push(slot);
            Some(slot)
        } else {
            None
        };
        let mut leaves = Vec::new();
        for ((is_mut, shape), (arg, mty)) in entry.params.iter().zip(call.args.iter().zip(params)) {
            match (is_mut, shape, arg) {
                (false, Shape::Int(_), Arg::Int(n)) => {
                    let i = match mty {
                        cfg::Ty::Int(i) => *i,
                        _ => return Err(err("an integer argument of another type")),
                    };
                    if !i.holds(*n) {
                        return Err(err(format!("the argument {} lies outside `{}`", n, i)));
                    }
                    let v = e.int(int_type(i), *n, i);
                    flat.extend(e.split(v, int_type(i)));
                }
                (false, Shape::Bool, Arg::Bool(x)) => flat.push(e.iconst(types::I8, i64::from(*x))),
                (_, shape, arg) => {
                    let at = shared.take(&mut e, shape.size())?;
                    store_arg(&mut e, at, shape, arg)?;
                    flat.push(at);
                    if *is_mut {
                        leaves.push((at, shape.clone()));
                    }
                }
            }
        }
        let fref = writer
            .module_mut()
            .declare_func_in_func(ids[entry.func.0 as usize], e.b.func);
        let inst = e.b.ins().call(fref, &flat);
        let results = e.b.inst_results(inst).to_vec();
        match (sret, &entry.result) {
            (Some(at), shape) => print_at(writer, rt, &mut e, at, shape)?,
            (None, Shape::Int(t)) => {
                let i = crate::lower::mir(*t);
                let v = match wide {
                    Some(at) => {
                        let fl = trusted(e.b);
                        e.b.ins().load(types::I128, fl, at, 0)
                    }
                    None => results[0],
                };
                emit_num(writer, rt, &mut e, 1, v, i);
            }
            (None, Shape::Bool) => print_bool(writer, rt, &mut e, results[0])?,
            _ => emit_text(writer, rt, &mut e, 1, "()")?,
        }
        emit_text(writer, rt, &mut e, 1, "\n")?;
        for (at, shape) in leaves {
            emit_text(writer, rt, &mut e, 1, "leaves ")?;
            print_at(writer, rt, &mut e, at, &shape)?;
            emit_text(writer, rt, &mut e, 1, "\n")?;
        }
        let zero = e.iconst(types::I32, 0);
        e.b.ins().return_(&[zero]);
    }
    Ok(())
}

/// The stack space `main`'s calls keep their arguments and results in. One call runs in a
/// process, so every call takes its space from the start of one slot, as large as the most any
/// call takes, rather than a slot of its own: `main`'s frame is then one call's, not the sum of
/// them all, which a few calls with a 64 KiB array each would take past a 1 MiB stack.
struct Shared {
    slot: ir::StackSlot,
    /// The bytes the current call has taken.
    used: u32,
    /// The most bytes any call has taken, the slot's size once every call is made.
    most: u32,
}

impl Shared {
    /// The address of `bytes` more of the slot for the current call, 16-byte aligned.
    fn take(&mut self, e: &mut Emit, bytes: u32) -> Result<Value, CodegenError> {
        let at = self.used;
        let end = bytes
            .max(1)
            .checked_next_multiple_of(16)
            .and_then(|n| at.checked_add(n))
            .filter(|&n| i32::try_from(n).is_ok())
            .ok_or_else(|| err("the arguments of one call take more than 2 GiB"))?;
        self.used = end;
        self.most = self.most.max(end);
        Ok(e.b.ins().stack_addr(e.ptr, self.slot, at as i32))
    }
}

fn slot(e: &mut Emit, bytes: u32) -> Value {
    let s = e.b.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        bytes.max(1),
        4,
    ));
    e.b.ins().stack_addr(e.ptr, s, 0)
}

/// Stores an argument of `shape` at `at`.
fn store_arg(e: &mut Emit, at: Value, shape: &Shape, arg: &Arg) -> Result<(), CodegenError> {
    match (shape, arg) {
        (Shape::Int(t), Arg::Int(n)) => {
            let i = crate::lower::mir(*t);
            if !i.holds(*n) {
                return Err(err(format!("the argument {} lies outside `{}`", n, t.name)));
            }
            let v = e.int(int_type(i), *n, i);
            let fl = trusted(e.b);
            e.b.ins().store(fl, v, at, 0);
        }
        (Shape::Bool, Arg::Bool(x)) => {
            let v = e.iconst(types::I8, i64::from(*x));
            let fl = trusted(e.b);
            e.b.ins().store(fl, v, at, 0);
        }
        (Shape::Array(len, elem), Arg::Array(items)) if items.len() == *len as usize => {
            for (k, item) in items.iter().enumerate() {
                let off = e.iconst(e.ptr, i64::from(elem.size()) * k as i64);
                let a = e.b.ins().iadd(at, off);
                store_arg(e, a, elem, item)?;
            }
        }
        _ => return Err(err("an argument of another shape than its parameter")),
    }
    Ok(())
}

fn print_bool(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    e: &mut Emit,
    v: Value,
) -> Result<(), CodegenError> {
    let yes = e.b.create_block();
    let no = e.b.create_block();
    let after = e.b.create_block();
    e.b.ins().brif(v, yes, &[], no, &[]);
    e.b.switch_to_block(yes);
    emit_text(writer, rt, e, 1, "true")?;
    e.b.ins().jump(after, &[]);
    e.b.switch_to_block(no);
    emit_text(writer, rt, e, 1, "false")?;
    e.b.ins().jump(after, &[]);
    e.b.switch_to_block(after);
    Ok(())
}

/// Writes the value of `shape` at `at` to standard output, as the reference interpreter does.
fn print_at(
    writer: &mut ObjectWriter,
    rt: &mut Runtime,
    e: &mut Emit,
    at: Value,
    shape: &Shape,
) -> Result<(), CodegenError> {
    match shape {
        Shape::Int(t) => {
            let i = crate::lower::mir(*t);
            let v = {
                let fl = trusted(e.b);
                e.b.ins().load(int_type(i), fl, at, 0)
            };
            emit_num(writer, rt, e, 1, v, i);
        }
        Shape::Bool => {
            let v = {
                let fl = trusted(e.b);
                e.b.ins().load(types::I8, fl, at, 0)
            };
            print_bool(writer, rt, e, v)?;
        }
        Shape::Array(len, elem) => {
            emit_text(writer, rt, e, 1, "[")?;
            for k in 0..*len {
                if k > 0 {
                    emit_text(writer, rt, e, 1, ", ")?;
                }
                let off = e.iconst(e.ptr, i64::from(elem.size()) * i64::from(k));
                let a = e.b.ins().iadd(at, off);
                print_at(writer, rt, e, a, elem)?;
            }
            emit_text(writer, rt, e, 1, "]")?;
        }
        Shape::Unit => emit_text(writer, rt, e, 1, "()")?,
    }
    Ok(())
}

/// The object of a program for `target`: every function of `lowered`, the runtime and a `main`
/// that makes the call `argv[1]` numbers among `calls`, compiled for `target`'s baseline
/// processor. `source` names the `.mn` file in the debug information, and `object_name` the
/// object's own file name, which CodeView records; `prefix` starts every trap's message.
///
/// Its bytes depend on these arguments alone: compiling twice, or on another machine, gives the
/// same object.
pub fn object(
    lowered: &Lowered,
    calls: &[Call],
    prefix: &str,
    source: crate::SourceFile,
    object_name: &str,
    target: Target,
) -> Result<Vec<u8>, CodegenError> {
    let isa = target.isa(
        cranelift_codegen::settings::OptLevel::Speed,
        crate::Cpu::Baseline,
    )?;
    let mut writer = ObjectWriter::new(isa, source, object_name)?;
    program(lowered, calls, prefix, &mut writer)?;
    writer.finish()
}

/// Builds the executable `exe` for the host: the program's [`object`], written beside `exe`,
/// and linked.
pub fn executable(
    lowered: &Lowered,
    calls: &[Call],
    prefix: &str,
    source: crate::SourceFile,
    exe: &std::path::Path,
) -> Result<(), CodegenError> {
    let target = Target::host().ok_or_else(|| err("no native code for this host"))?;
    let path = object_path(exe, target);
    let name = path
        .file_name()
        .map_or_else(|| "program.o".into(), |n| n.to_string_lossy().into_owned());
    let bytes = object(lowered, calls, prefix, source, &name, target)?;
    std::fs::write(&path, bytes).map_err(|e| err(format!("{}: {e}", path.display())))?;
    crate::link::link(&path, exe, target)
}

/// The object file `executable` writes beside `exe`, with the extension of `target`'s format.
pub fn object_path(exe: &std::path::Path, target: Target) -> std::path::PathBuf {
    exe.with_extension(match target {
        Target::X86_64Windows => "obj",
        Target::X86_64Linux | Target::Aarch64Linux => "o",
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use cranelift_codegen::settings::OptLevel;
    use cranelift_module::FuncOrDataId;

    use super::*;
    use crate::{Cpu, SourceFile};

    const SOURCE: &str = "module m
pub fn f(a: u8, b: u8) -> u8 {
  val s: u16 = a + b
  s
}
pub fn g(xs: [3]u8) -> u8 = xs[0]
";

    fn lowered() -> Lowered {
        let module = mente_sema::check_program_without_proofs(SOURCE).unwrap();
        crate::lower::lower(&module).unwrap()
    }

    fn writer(target: Target) -> ObjectWriter {
        let isa = target.isa(OptLevel::Speed, Cpu::Baseline).unwrap();
        let source = SourceFile {
            path: "m.mn".into(),
            directory: ".".into(),
        };
        ObjectWriter::new(isa, source, "m.o").unwrap()
    }

    /// Each MIR function becomes a function named after it, declared at its first line, whose
    /// instructions carry the lines they come from.
    #[test]
    fn test_native_functions_carry_their_names_and_source_lines() {
        let lowered = lowered();
        let index = lowered
            .module
            .functions
            .iter()
            .position(|f| f.name == "f")
            .unwrap();
        for target in Target::ALL {
            let mut w = writer(target);
            program(&lowered, &[], "", &mut w).unwrap();
            let Some(FuncOrDataId::Func(id)) = w.module().get_name(&format!("mente_f{index}_f"))
            else {
                panic!("{target:?}: no function named after `f`");
            };
            let compiled = w.functions.iter().find(|c| c.id == id).unwrap();
            assert_eq!(compiled.debug_name.as_deref(), Some("f"), "{target:?}");
            assert_eq!(compiled.decl.line, 2, "{target:?}");
            let lines: BTreeSet<u32> = compiled.ranges.iter().map(|r| r.position.line).collect();
            assert!(
                lines.contains(&3) && lines.contains(&4),
                "{target:?}: {lines:?}"
            );
            assert!(
                lines.iter().all(|l| (2..=4).contains(l)),
                "{target:?}: {lines:?}"
            );
        }
    }

    /// A call's argument must have its parameter's shape: an array of its length, of its
    /// element type.
    #[test]
    fn test_an_argument_must_have_its_parameters_shape() {
        let lowered = lowered();
        let g = lowered.entries.iter().position(|e| e.name == "g").unwrap();
        let int = |v: i128| Arg::Int(Num::from_i128(v));
        let build = |args: Vec<Arg>| {
            let mut w = writer(Target::X86_64Linux);
            program(&lowered, &[Call { entry: g, args }], "", &mut w)
        };
        assert!(build(vec![Arg::Array(vec![int(1), int(2), int(3)])]).is_ok());
        for wrong in [
            vec![Arg::Array(vec![int(1), int(2)])],
            vec![Arg::Array(vec![int(1), int(2), int(3), int(4)])],
            vec![Arg::Array(vec![int(1), Arg::Bool(true), int(3)])],
            vec![int(1)],
        ] {
            assert!(build(wrong).is_err());
        }
    }

    /// Every call in `main` keeps its arguments in the same stack space, since only one runs:
    /// its frame holds one call's array, not one per call, which a few dozen calls of 64 KiB
    /// arrays would take past a 1 MiB stack.
    #[test]
    fn test_mains_calls_share_their_stack_space() {
        let source = "module m\npub fn first(xs: [4096]u8) -> u8 = xs[0]\n";
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        let zeros = Arg::Array(vec![Arg::Int(Num::from_i128(0)); 4096]);
        let calls: Vec<Call> = (0..32)
            .map(|_| Call {
                entry: 0,
                args: vec![zeros.clone()],
            })
            .collect();
        let mut w = writer(Target::X86_64Windows);
        program(&lowered, &calls, "", &mut w).unwrap();
        let Some(FuncOrDataId::Func(id)) = w.module().get_name("main") else {
            panic!("no `main`");
        };
        let main = w.functions.iter().find(|c| c.id == id).unwrap();
        assert!(
            (4096..2 * 4096).contains(&main.stack_slots),
            "{}",
            main.stack_slots
        );
    }

    #[test]
    fn test_the_object_lies_beside_the_executable_in_its_targets_format() {
        let dir = Path::new("out");
        assert_eq!(
            object_path(&dir.join("prog.exe"), Target::X86_64Windows),
            dir.join("prog.obj")
        );
        for target in [Target::X86_64Linux, Target::Aarch64Linux] {
            assert_eq!(object_path(&dir.join("prog"), target), dir.join("prog.o"));
        }
    }

    /// A divisor is a constant when it is an `int`, or a cast of one into a type that holds it;
    /// anything else, a cast that would change it included, is not.
    #[test]
    fn test_a_constant_divisor_is_seen_through_casts_that_keep_its_value() {
        let (i64t, u128t) = (IntTy::new(64, true), IntTy::new(128, false));
        let mut b = cfg::Builder::new("f", &[cfg::Ty::Int(u128t)], None);
        let p = b.params(cfg::BlockId(0))[0];
        let two = b.int(i64t, Num::from_i128(2));
        let wide = b.inst(Op::Cast, cfg::Ty::Int(u128t), &[two]);
        let minus = b.int(i64t, Num::from_i128(-3));
        let wrapped = b.inst(Op::Cast, cfg::Ty::Int(u128t), &[minus]);
        let sum = b.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: None,
            },
            cfg::Ty::Int(u128t),
            &[p, wide],
        );
        let f = b.finish();
        assert_eq!(constant(&f, two), Some(Num::from_i128(2)));
        assert_eq!(constant(&f, wide), Some(Num::from_i128(2)));
        assert_eq!(constant(&f, minus), Some(Num::from_i128(-3)));
        assert_eq!(constant(&f, wrapped), None);
        assert_eq!(constant(&f, p), None);
        assert_eq!(constant(&f, sum), None);
    }

    #[test]
    fn test_a_power_of_two_divisor_divides_by_a_shift() {
        assert_eq!(power_of_two(Some(1)), Some(0));
        assert_eq!(power_of_two(Some(2)), Some(1));
        assert_eq!(power_of_two(Some(8)), Some(3));
        assert_eq!(power_of_two(Some(1 << 127)), Some(127));
        assert_eq!(power_of_two(Some(10)), None);
        assert_eq!(power_of_two(Some(0)), None);
        assert_eq!(power_of_two(None), None);
    }

    /// A call from outside reports the first argument, in order and element by element, that its
    /// parameter's type cannot hold, as the reference interpreter's narrowing at the function's
    /// name does, whether or not the entry's MIR parameter could hold it.
    #[test]
    fn test_an_argument_outside_its_type_traps_as_the_reference_interpreter_does() {
        let source = "module m
pub fn f(a: i128, xs: [2]u8, mut b: u128) -> i128 {
  b = 0
  a + xs[0]
}
";
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        let entry = lowered.entries.iter().find(|e| e.name == "f").unwrap();
        let int = |n: i128| Arg::Int(Num::from_i128(n));
        let args = |a: Arg, xs: [i128; 2], b: Arg| vec![a, Arg::Array(xs.map(int).to_vec()), b];
        let below_i128 = Arg::Int(Num::new(true, (1u128 << 127) + 1));
        assert_eq!(refusal(entry, &args(int(1), [2, 3], int(4))), None);
        assert_eq!(
            refusal(entry, &args(below_i128.clone(), [2, 3], int(4))).as_deref(),
            Some("trap[narrowing] at 2:8: -170141183460469231731687303715884105729 does not fit in `i128`")
        );
        // The first value that does not fit, though a later one does not either.
        assert_eq!(
            refusal(entry, &args(int(1), [300, -1], int(-1))).as_deref(),
            Some("trap[narrowing] at 2:8: 300 does not fit in `u8`")
        );
        assert_eq!(
            refusal(entry, &args(int(1), [2, 3], int(-1))).as_deref(),
            Some("trap[narrowing] at 2:8: -1 does not fit in `u128`")
        );
        // An argument of another shape is not a narrowing; the caller refuses it otherwise.
        assert_eq!(refusal(entry, &args(Arg::Bool(true), [2, 3], int(4))), None);
    }

    /// The symbols of an object that are not the program's own functions: what it imports, and
    /// each function it defines whose name does not start with `mente_f`.
    fn platform_symbols(bytes: &[u8]) -> BTreeSet<String> {
        use object::{Object as _, ObjectSymbol as _, SymbolKind};
        let file = object::File::parse(bytes).unwrap();
        file.symbols()
            .filter(|s| s.is_undefined() || (s.is_definition() && s.kind() == SymbolKind::Text))
            .map(|s| s.name().unwrap().to_string())
            .filter(|name| !name.is_empty() && !name.starts_with("mente_f"))
            .collect()
    }

    /// Every function and import of a native program beyond its Mente functions is listed in
    /// the platform layer, and every symbol listed there is in some target's object.
    #[test]
    fn test_every_symbol_outside_the_program_is_listed_in_the_platform_layer() {
        let doc = include_str!("../../../docs/platform-layer.md");
        let listed: BTreeSet<String> = doc
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split_once('`'))
            .map(|(name, _)| name.to_string())
            .collect();
        // A copy of an array, a 128-bit division and a trap, so that every part is used.
        let source = "module m
pub fn f(xs: [8]u64, d: u128) -> u64 {
  var ys: [8]u64 = xs
  ys[0] = 1
  val q: u64 = 1000 / d
  ys[0] + q
}
";
        let module = mente_sema::check_program_without_proofs(source).unwrap();
        let lowered = crate::lower::lower(&module).unwrap();
        let mut seen = BTreeSet::new();
        for target in Target::ALL {
            let mut w = writer(target);
            program(&lowered, &[], "", &mut w).unwrap();
            let symbols = platform_symbols(&w.finish().unwrap());
            for name in &symbols {
                assert!(
                    listed.contains(name),
                    "{target:?}: `{name}` is in the object but not in docs/platform-layer.md"
                );
            }
            seen.extend(symbols);
        }
        for name in &listed {
            assert!(
                seen.contains(name),
                "docs/platform-layer.md lists `{name}`, which no object has"
            );
        }
    }
}

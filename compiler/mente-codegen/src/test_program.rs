//! The program the tests compile: the M0b spike's `demo.mn`, written by hand in Cranelift IR
//! with a source position on every instruction, compiled for each target.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature, UserFuncName};
use cranelift_codegen::settings::OptLevel;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};

use crate::{CompiledFunction, Cpu, ObjectWriter, SourceFile, SourcePos, Target};

pub(crate) const SOURCE: &str = "\
// The program the tests compile, written by hand as Cranelift IR in test_program.rs. Its line
// table, symbols and unwind information point here.

fn square(x: i64) -> i64 {
    return x * x
}

fn sum_squares(n: i64) -> i64 {
    var total: i64 = 0
    var i: i64 = 1
    while i <= n {
        total += square(i)
        i += 1
    }
    return total
}

fn mente_main() -> i64 {
    return sum_squares(10)
}
";

/// Each function, in the order it is defined, with the text its declaration starts with and
/// then the text at each source position of its body.
pub(crate) const FUNCTIONS: &[(&str, &[&str])] = &[
    ("square", &["fn square", "x * x", "return x * x"]),
    (
        "sum_squares",
        &[
            "fn sum_squares",
            "var total: i64 = 0",
            "var i: i64 = 1",
            "while i <= n",
            "square(i)",
            "total += square(i)",
            "i += 1",
            "return total",
        ],
    ),
    (
        "mente_main",
        &["fn mente_main", "sum_squares(10)", "return sum_squares(10)"],
    ),
];

/// Where `needle` starts in the source: its line and column, both from 1.
pub(crate) fn position_of(needle: &str) -> SourcePos {
    let (k, line) = SOURCE
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .unwrap_or_else(|| panic!("the source has no {needle:?}"));
    SourcePos {
        line: k as u32 + 1,
        column: line.find(needle).unwrap() as u32 + 1,
    }
}

/// The position of a function's declaration.
pub(crate) fn decl_of(name: &str) -> SourcePos {
    let (_, needles) = FUNCTIONS.iter().find(|f| f.0 == name).unwrap();
    position_of(needles[0])
}

pub(crate) fn source_file() -> SourceFile {
    SourceFile {
        path: "demo.mn".to_string(),
        directory: "/src".to_string(),
    }
}

/// A writer for `target` with nothing defined, compiling without optimisation so that every
/// source position keeps its code.
pub(crate) fn empty_writer(target: Target) -> ObjectWriter {
    let isa = target.isa(OptLevel::None, Cpu::Baseline).unwrap();
    let object_name = match target {
        Target::X86_64Windows => "demo.obj",
        Target::X86_64Linux | Target::Aarch64Linux => "demo.o",
    };
    ObjectWriter::new(isa, source_file(), object_name).unwrap()
}

fn signature(writer: &ObjectWriter, params: usize) -> Signature {
    let mut sig = writer.module().make_signature();
    for _ in 0..params {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    sig
}

fn context(sig: &Signature, id: FuncId) -> Context {
    let mut ctx = Context::new();
    ctx.func.signature = sig.clone();
    ctx.func.name = UserFuncName::user(0, id.as_u32());
    ctx
}

/// A writer for `target` with the program's three functions defined.
pub(crate) fn writer(target: Target) -> ObjectWriter {
    let mut w = empty_writer(target);
    let sig1 = signature(&w, 1);
    let sig0 = signature(&w, 0);
    let declare = |w: &mut ObjectWriter, name: &str, sig: &Signature| {
        w.module_mut()
            .declare_function(name, Linkage::Export, sig)
            .unwrap()
    };
    let square = declare(&mut w, "square", &sig1);
    let sum_squares = declare(&mut w, "sum_squares", &sig1);
    let mente_main = declare(&mut w, "mente_main", &sig0);
    let frontend = w.module().isa().frontend_config();
    let at = |w: &mut ObjectWriter, needle: &str| w.source_loc(position_of(needle)).unwrap();
    let mut fctx = FunctionBuilderContext::new();

    // fn square(x: i64) -> i64 { return x * x }
    {
        let mut ctx = context(&sig1, square);
        let (product, ret) = (at(&mut w, "x * x"), at(&mut w, "return x * x"));
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let x = b.block_params(entry)[0];
        b.set_srcloc(product);
        let r = b.ins().imul(x, x);
        b.set_srcloc(ret);
        b.ins().return_(&[r]);
        b.finalize(frontend);
        w.define_function(square, &mut ctx, decl_of("square"))
            .unwrap();
    }

    // fn sum_squares(n: i64) -> i64 { var total = 0; var i = 1; while i <= n { total += square(i);
    // i += 1 }; return total }
    {
        let mut ctx = context(&sig1, sum_squares);
        let callee = w.module_mut().declare_func_in_func(square, &mut ctx.func);
        let locs: Vec<_> = FUNCTIONS[1].1[1..]
            .iter()
            .map(|needle| at(&mut w, needle))
            .collect();
        let [total0, i1, cond, call, add, step, ret] = locs[..] else {
            panic!("sum_squares has seven positions");
        };
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let (entry, head, body, exit) = (
            b.create_block(),
            b.create_block(),
            b.create_block(),
            b.create_block(),
        );
        let total = b.declare_var(types::I64);
        let i = b.declare_var(types::I64);
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let n = b.block_params(entry)[0];
        b.set_srcloc(total0);
        let zero = b.ins().iconst(types::I64, 0);
        b.def_var(total, zero);
        b.set_srcloc(i1);
        let one = b.ins().iconst(types::I64, 1);
        b.def_var(i, one);
        b.ins().jump(head, &[]);
        b.switch_to_block(head);
        b.set_srcloc(cond);
        let iv = b.use_var(i);
        let c = b.ins().icmp(IntCC::SignedLessThanOrEqual, iv, n);
        b.ins().brif(c, body, &[], exit, &[]);
        b.switch_to_block(body);
        b.seal_block(body);
        b.set_srcloc(call);
        let iv = b.use_var(i);
        let called = b.ins().call(callee, &[iv]);
        let sq = b.inst_results(called)[0];
        b.set_srcloc(add);
        let tv = b.use_var(total);
        let t2 = b.ins().iadd(tv, sq);
        b.def_var(total, t2);
        b.set_srcloc(step);
        let iv = b.use_var(i);
        let one = b.ins().iconst(types::I64, 1);
        let i2 = b.ins().iadd(iv, one);
        b.def_var(i, i2);
        b.ins().jump(head, &[]);
        b.seal_block(head);
        b.switch_to_block(exit);
        b.seal_block(exit);
        b.set_srcloc(ret);
        let tv = b.use_var(total);
        b.ins().return_(&[tv]);
        b.finalize(frontend);
        w.define_function(sum_squares, &mut ctx, decl_of("sum_squares"))
            .unwrap();
    }

    // fn mente_main() -> i64 { return sum_squares(10) }
    {
        let mut ctx = context(&sig0, mente_main);
        let callee = w
            .module_mut()
            .declare_func_in_func(sum_squares, &mut ctx.func);
        let (call, ret) = (
            at(&mut w, "sum_squares(10)"),
            at(&mut w, "return sum_squares(10)"),
        );
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.switch_to_block(entry);
        b.seal_block(entry);
        b.set_srcloc(call);
        let ten = b.ins().iconst(types::I64, 10);
        let called = b.ins().call(callee, &[ten]);
        let r = b.inst_results(called)[0];
        b.set_srcloc(ret);
        b.ins().return_(&[r]);
        b.finalize(frontend);
        w.define_function(mente_main, &mut ctx, decl_of("mente_main"))
            .unwrap();
    }
    w
}

/// The program's object for `target`, and what the writer recorded about each function.
pub(crate) fn object(target: Target) -> (Vec<u8>, Vec<CompiledFunction>) {
    let w = writer(target);
    let functions = w.functions.clone();
    (w.finish().unwrap(), functions)
}

/// A writer for `target` with one function, `fn one() -> i64 { return 1 }`, declared and
/// compiled at `position`.
pub(crate) fn writer_with_one(target: Target, position: SourcePos) -> ObjectWriter {
    let mut w = empty_writer(target);
    let sig = signature(&w, 0);
    let id = w
        .module_mut()
        .declare_function("one", Linkage::Export, &sig)
        .unwrap();
    let frontend = w.module().isa().frontend_config();
    let loc = w.source_loc(position).unwrap();
    let mut ctx = context(&sig, id);
    let mut fctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
    let entry = b.create_block();
    b.switch_to_block(entry);
    b.seal_block(entry);
    b.set_srcloc(loc);
    let one = b.ins().iconst(types::I64, 1);
    b.ins().return_(&[one]);
    b.finalize(frontend);
    w.define_function(id, &mut ctx, position).unwrap();
    w
}

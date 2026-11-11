//! Spike before M2: Mente's `T ! E` returned in registers through Cranelift's multi-value
//! returns (README.md).
//!
//! A fallible function returns a tag, an integer that is 0 for success and 1 for an error, and
//! a payload of one or more 64-bit words that hold either `T` or `E`. The spike writes, by hand
//! in Cranelift IR, chains of three such functions in which each caller tests the tag after the
//! call and returns an error unchanged, as postfix `!` does (it only propagates an identical
//! error type), in three styles:
//!
//! - [`Style::Platform`]: tag and payload words as Cranelift return values, in the target's
//!   default calling convention (System V on Linux, fastcall on Windows);
//! - [`Style::Tail`]: the same in Cranelift's own `tail` convention, which has more return
//!   registers on x86-64;
//! - [`Style::Buffer`]: the tag is the one return value, and the payload is written to a buffer
//!   whose address the caller passes as an ordinary parameter.
//!
//! Every chain gets an exported wrapper in the C convention, `<prefix>_run(x, out)`, which
//! calls the chain's outermost function and stores the tag and the payload words at `out`, so
//! that a host program can check the round trip on every target. A probe per chain,
//! `<prefix>_probe`, returns a distinct constant in every position, so that the listing shows
//! which register (or memory) each position travels in.

use std::fmt::Write as _;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, ArgumentPurpose, FuncRef, InstBuilder, MemFlagsData, Signature, Type,
    UserFuncName, Value,
};
use cranelift_codegen::isa::unwind::UnwindInfo;
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

/// The three targets Mente's native code runs on.
pub const TARGETS: [&str; 3] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// How a chain returns its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Tag and payload words as return values, in the target's default calling convention.
    Platform,
    /// Tag and payload words as return values, in Cranelift's `tail` convention.
    Tail,
    /// The tag as the one return value; the payload goes to a buffer the caller provides.
    Buffer,
}

/// One chain of three fallible functions: its symbol prefix, the number of 64-bit payload
/// words (the larger of `T` and `E`), how it returns them, and the tag's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    pub prefix: &'static str,
    pub words: usize,
    pub style: Style,
    pub tag: Type,
}

const fn chain(prefix: &'static str, words: usize, style: Style, tag: Type) -> Chain {
    Chain {
        prefix,
        words,
        style,
        tag,
    }
}

/// `i64 ! u64` with an `i8` tag: tag and an 8-byte payload, the shape Rust returns as
/// `Result<u64, u64>`.
pub const PAIR: Chain = chain("pair", 1, Style::Platform, types::I8);
/// The same with an `i32` tag, which AArch64 tests without masking it first.
pub const PAIR32: Chain = chain("pair32", 1, Style::Platform, types::I32);
/// A 16-byte payload (an `E` of two words) as return values in the platform convention.
pub const WIDE_PLATFORM: Chain = chain("wide_platform", 2, Style::Platform, types::I8);
/// A 24-byte payload (a `T` of three words) as return values in the platform convention.
pub const BIG_PLATFORM: Chain = chain("big_platform", 3, Style::Platform, types::I8);
/// The 16-byte payload as return values in the `tail` convention.
pub const WIDE_TAIL: Chain = chain("wide_tail", 2, Style::Tail, types::I8);
/// The 24-byte payload as return values in the `tail` convention.
pub const BIG_TAIL: Chain = chain("big_tail", 3, Style::Tail, types::I8);
/// The 16-byte payload in a caller's buffer, the tag in a register.
pub const WIDE_BUFFER: Chain = chain("wide_buffer", 2, Style::Buffer, types::I8);
/// The 24-byte payload in a caller's buffer, the tag in a register.
pub const BIG_BUFFER: Chain = chain("big_buffer", 3, Style::Buffer, types::I8);
/// The 16-byte payload as return values in the platform convention, with Cranelift's
/// deprecated `enable_multi_ret_implicit_sret`, built into an object of its own.
pub const WIDE_IMPLICIT: Chain = chain("wide_implicit", 2, Style::Platform, types::I8);
/// The 24-byte payload as return values with `enable_multi_ret_implicit_sret`.
pub const BIG_IMPLICIT: Chain = chain("big_implicit", 3, Style::Platform, types::I8);

/// The chains of the main object, compiled with the flags `mente-codegen` sets. A chain the
/// target refuses is reported and left out.
pub const MAIN_CHAINS: [Chain; 8] = [
    PAIR,
    PAIR32,
    WIDE_PLATFORM,
    BIG_PLATFORM,
    WIDE_TAIL,
    BIG_TAIL,
    WIDE_BUFFER,
    BIG_BUFFER,
];
/// The chains of the second object, compiled with `enable_multi_ret_implicit_sret` too.
pub const IMPLICIT_CHAINS: [Chain; 2] = [WIDE_IMPLICIT, BIG_IMPLICIT];

/// The spike's rules for the payload, which the host program recomputes on its own:
/// - `leaf(x)` raises `1000 - x` when `x < 0`, and otherwise returns `2x`;
/// - `middle(x)` propagates `leaf(x)`'s error, raises `2000 + v` when `v > 100`, and otherwise
///   returns `v + 1`;
/// - `outer(x)` propagates `middle(x)`'s error, raises `3000 + v` when `v % 5 == 0`, and
///   otherwise returns `10v`;
///
/// where a payload's word `i` is `code + i * ERROR_WORD_STEP` for an error and `2x + i`
/// (`leaf`), `w[i] + 1` (`middle`) or `10 w[i]` (`outer`) for a success.
pub const ERROR_WORD_STEP: i64 = 100_000;

/// The constant a probe returns as its tag.
pub const PROBE_TAG: i64 = 0x11;
/// The constant a probe returns as payload word `i`.
pub fn probe_word(i: usize) -> i64 {
    0x2201 + i as i64
}

/// Cranelift's `opt_level` settings the spike compiles at.
pub const OPT_LEVELS: [&str; 2] = ["none", "speed"];

/// A Cranelift ISA for `triple` (the machine running the spike when `None`), with the flags
/// `mente-codegen` sets: position-independent code, inline stack probes, and `opt_level`. With
/// `implicit_sret`, Cranelift's deprecated `enable_multi_ret_implicit_sret` too.
pub fn isa(
    triple: Option<&str>,
    opt_level: &str,
    implicit_sret: bool,
) -> Result<OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| {
        f.set(k, v).map_err(|e| format!("{k}={v}: {e}"))
    };
    set(&mut flags, "opt_level", opt_level)?;
    set(&mut flags, "is_pic", "true")?;
    set(&mut flags, "enable_probestack", "true")?;
    set(&mut flags, "probestack_strategy", "inline")?;
    if implicit_sret {
        set(&mut flags, "enable_multi_ret_implicit_sret", "true")?;
    }
    let builder = match triple {
        None => cranelift_native::builder().map_err(|e| e.to_string())?,
        Some(t) => cranelift_codegen::isa::lookup_by_name(t).map_err(|e| format!("{t}: {e}"))?,
    };
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}

/// What the spike records about one compiled function.
#[derive(Clone, Debug)]
pub struct CompiledFn {
    pub name: String,
    pub call_conv: CallConv,
    pub size: usize,
    /// Cranelift's own listing of the function's machine code, with physical registers.
    pub disasm: String,
    /// The unwind information Cranelift makes for the function, as an object writer asks for
    /// it: "System V CFI", "Windows x64", "none", or the error.
    pub unwind: String,
}

/// An object built for one ISA: its bytes, its functions, and the chains the ISA refused,
/// each with Cranelift's error.
#[derive(Debug)]
pub struct Built {
    pub object: Vec<u8>,
    pub functions: Vec<CompiledFn>,
    pub refused: Vec<(Chain, String)>,
}

impl Built {
    /// The compiled function called `name`, if the object has it.
    pub fn function(&self, name: &str) -> Option<&CompiledFn> {
        self.functions.iter().find(|f| f.name == name)
    }
}

/// Builds an object holding `chains`, each with its wrapper and its probe, for `isa`; with
/// `extras`, also the functions that compare `!` against a plain call ([`build_forward`]).
pub fn build_object(
    isa: OwnedTargetIsa,
    name: &str,
    chains: &[Chain],
    extras: bool,
) -> Result<Built, String> {
    let mut refused = Vec::new();
    let mut accepted = Vec::new();
    for c in chains {
        // A chain the ISA refuses would leave declared symbols undefined, so each is first
        // compiled into a scratch module of its own.
        let mut scratch = new_module(isa.clone(), "scratch")?;
        match build_chain(&mut scratch, *c) {
            Ok(_) => accepted.push(*c),
            Err(e) => refused.push((*c, e)),
        }
    }
    let mut module = new_module(isa, name)?;
    let mut functions = Vec::new();
    for c in &accepted {
        functions.extend(build_chain(&mut module, *c)?);
    }
    if extras {
        functions.extend(build_plain(&mut module)?);
        for c in [PAIR, PAIR32] {
            if accepted.contains(&c) {
                functions.extend(build_forward(&mut module, c)?);
            }
        }
    }
    let object = module.finish().emit().map_err(|e| e.to_string())?;
    Ok(Built {
        object,
        functions,
        refused,
    })
}

fn new_module(isa: OwnedTargetIsa, name: &str) -> Result<ObjectModule, String> {
    let mut builder = ObjectBuilder::new(isa, name, cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    // cranelift-object's .eh_frame cannot be linked into a position-independent executable
    // (spikes/cranelift-debug-info, finding 2), and the round trip needs no unwinding; the
    // unwind information is still made and checked per function.
    builder.unwind_info(false);
    Ok(ObjectModule::new(builder))
}

/// The calling convention of a chain's functions on this module's target.
fn chain_conv(module: &ObjectModule, style: Style) -> CallConv {
    match style {
        Style::Platform | Style::Buffer => module.isa().default_call_conv(),
        Style::Tail => CallConv::Tail,
    }
}

/// The signature of a chain's fallible functions.
fn chain_sig(module: &ObjectModule, c: Chain) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut sig = Signature::new(chain_conv(module, c.style));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(c.tag));
    match c.style {
        Style::Platform | Style::Tail => {
            for _ in 0..c.words {
                sig.returns.push(AbiParam::new(types::I64));
            }
        }
        Style::Buffer => sig.params.push(AbiParam::new(ptr)),
    }
    sig
}

/// The signature of a `_run` wrapper: `extern "C" fn(x: i64, out: *mut u64)`.
fn wrapper_sig(module: &ObjectModule) -> Signature {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params
        .push(AbiParam::new(module.target_config().pointer_type()));
    sig
}

fn declare(module: &mut ObjectModule, name: &str, sig: &Signature) -> Result<FuncId, String> {
    module
        .declare_function(name, Linkage::Export, sig)
        .map_err(|e| format!("{name}: {e}"))
}

/// Compiles one function: `body` receives the builder, positioned in the sealed entry block,
/// the references to `callees`, and the entry block's parameters.
fn define(
    module: &mut ObjectModule,
    name: &str,
    id: FuncId,
    sig: &Signature,
    callees: &[FuncId],
    body: impl FnOnce(&mut FunctionBuilder, &[FuncRef], &[Value]),
) -> Result<CompiledFn, String> {
    let mut ctx = Context::new();
    ctx.set_disasm(true);
    ctx.func.signature = sig.clone();
    ctx.func.name = UserFuncName::user(0, id.as_u32());
    let refs: Vec<FuncRef> = callees
        .iter()
        .map(|c| module.declare_func_in_func(*c, &mut ctx.func))
        .collect();
    let frontend = module.isa().frontend_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let params = b.block_params(entry).to_vec();
        body(&mut b, &refs, &params);
        b.seal_all_blocks();
        b.finalize(frontend);
    }
    module
        .define_function(id, &mut ctx)
        .map_err(|e| format!("{name}: {}", error_chain(&e)))?;
    let compiled = ctx
        .compiled_code()
        .ok_or_else(|| format!("{name}: no compiled code"))?;
    let unwind = match compiled.create_unwind_info(module.isa()) {
        Ok(Some(UnwindInfo::SystemV(_))) => "System V CFI".to_string(),
        Ok(Some(UnwindInfo::WindowsX64(_))) => "Windows x64".to_string(),
        Ok(Some(_)) => "other".to_string(),
        Ok(None) => "none".to_string(),
        Err(e) => format!("error: {e}"),
    };
    Ok(CompiledFn {
        name: name.to_string(),
        call_conv: sig.call_conv,
        size: compiled.code_buffer().len(),
        disasm: compiled.vcode.clone().unwrap_or_default(),
        unwind,
    })
}

/// An error and its sources, which `cranelift-module` keeps apart from its own message.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut source = e.source();
    while let Some(inner) = source {
        let _ = write!(s, ": {inner}");
        source = inner.source();
    }
    s
}

/// The words of a payload whose word `i` is `base + i * step`.
fn words(b: &mut FunctionBuilder, base: Value, n: usize, step: i64) -> Vec<Value> {
    (0..n)
        .map(|i| {
            if i == 0 {
                base
            } else {
                b.ins().iadd_imm_s(base, i as i64 * step)
            }
        })
        .collect()
}

/// Ends the current block by returning `tag` and `payload` in registers, or by storing the
/// payload in the buffer `out` and returning `tag`.
fn finish(b: &mut FunctionBuilder, c: Chain, out: Option<Value>, tag: i64, payload: &[Value]) {
    let tag = b.ins().iconst(c.tag, tag);
    match (c.style, out) {
        (Style::Buffer, Some(out)) => {
            for (i, w) in payload.iter().enumerate() {
                b.ins()
                    .store(MemFlagsData::trusted(), *w, out, (8 * i) as i32);
            }
            b.ins().return_(&[tag]);
        }
        _ => {
            let mut vals = vec![tag];
            vals.extend_from_slice(payload);
            b.ins().return_(&vals);
        }
    }
}

/// Calls a chain function and tests its tag: an error is returned unchanged, as postfix `!`
/// propagates it; on success, the builder is left in a new block and gets the payload.
fn call_and_propagate(
    b: &mut FunctionBuilder,
    c: Chain,
    callee: FuncRef,
    x: Value,
    out: Option<Value>,
) -> Vec<Value> {
    let (propagate, ok) = (b.create_block(), b.create_block());
    let mut args = vec![x];
    // A buffer chain hands its own buffer to the callee: `!` only propagates an identical
    // error type, so an error the callee writes is already where this function's caller
    // reads it, and propagating it is returning the tag.
    args.extend(out);
    let call = b.ins().call(callee, &args);
    let results = b.inst_results(call).to_vec();
    b.ins().brif(results[0], propagate, &[], ok, &[]);
    b.switch_to_block(propagate);
    b.ins().return_(&results);
    b.switch_to_block(ok);
    match (c.style, out) {
        (Style::Buffer, Some(out)) => (0..c.words)
            .map(|i| {
                b.ins()
                    .load(types::I64, MemFlagsData::trusted(), out, (8 * i) as i32)
            })
            .collect(),
        _ => results[1..].to_vec(),
    }
}

/// Builds a chain's `leaf`, `middle` and `outer`, its `_run` wrapper and its `_probe` into
/// `module`.
pub fn build_chain(module: &mut ObjectModule, c: Chain) -> Result<Vec<CompiledFn>, String> {
    let sig = chain_sig(module, c);
    let wsig = wrapper_sig(module);
    let p = c.prefix;
    let leaf = declare(module, &format!("{p}_leaf"), &sig)?;
    let middle = declare(module, &format!("{p}_middle"), &sig)?;
    let outer = declare(module, &format!("{p}_outer"), &sig)?;
    let run = declare(module, &format!("{p}_run"), &wsig)?;
    let probe = declare(module, &format!("{p}_probe"), &sig)?;
    let n = c.words;
    let mut out = Vec::new();

    // fn leaf(x: i64) -> W ! E { if x < 0 { raise 1000 - x }; return 2 * x }
    out.push(define(
        module,
        &format!("{p}_leaf"),
        leaf,
        &sig,
        &[],
        |b, _, ps| {
            let (x, buf) = (ps[0], ps.get(1).copied());
            let (err, ok) = (b.create_block(), b.create_block());
            let neg = b.ins().icmp_imm_s(IntCC::SignedLessThan, x, 0);
            b.ins().brif(neg, err, &[], ok, &[]);
            b.switch_to_block(err);
            let k = b.ins().iconst(types::I64, 1000);
            let code = b.ins().isub(k, x);
            let ws = words(b, code, n, ERROR_WORD_STEP);
            finish(b, c, buf, 1, &ws);
            b.switch_to_block(ok);
            let two_x = b.ins().iadd(x, x);
            let ws = words(b, two_x, n, 1);
            finish(b, c, buf, 0, &ws);
        },
    )?);

    // fn middle(x: i64) -> W ! E { val v = leaf(x)!; if v > 100 { raise 2000 + v }; return v + 1 }
    out.push(define(
        module,
        &format!("{p}_middle"),
        middle,
        &sig,
        &[leaf],
        |b, refs, ps| {
            let (x, buf) = (ps[0], ps.get(1).copied());
            let v = call_and_propagate(b, c, refs[0], x, buf);
            let (raise, done) = (b.create_block(), b.create_block());
            let big = b.ins().icmp_imm_s(IntCC::SignedGreaterThan, v[0], 100);
            b.ins().brif(big, raise, &[], done, &[]);
            b.switch_to_block(raise);
            let code = b.ins().iadd_imm_s(v[0], 2000);
            let ws = words(b, code, n, ERROR_WORD_STEP);
            finish(b, c, buf, 1, &ws);
            b.switch_to_block(done);
            let ws: Vec<Value> = v.iter().map(|w| b.ins().iadd_imm_s(*w, 1)).collect();
            finish(b, c, buf, 0, &ws);
        },
    )?);

    // fn outer(x: i64) -> W ! E { val v = middle(x)!; if v % 5 == 0 { raise 3000 + v }; return 10 * v }
    out.push(define(
        module,
        &format!("{p}_outer"),
        outer,
        &sig,
        &[middle],
        |b, refs, ps| {
            let (x, buf) = (ps[0], ps.get(1).copied());
            let v = call_and_propagate(b, c, refs[0], x, buf);
            let (raise, done) = (b.create_block(), b.create_block());
            let five = b.ins().iconst(types::I64, 5);
            let rem = b.ins().urem(v[0], five);
            b.ins().brif(rem, done, &[], raise, &[]);
            b.switch_to_block(raise);
            let code = b.ins().iadd_imm_s(v[0], 3000);
            let ws = words(b, code, n, ERROR_WORD_STEP);
            finish(b, c, buf, 1, &ws);
            b.switch_to_block(done);
            let ws: Vec<Value> = v.iter().map(|w| b.ins().imul_imm_s(*w, 10)).collect();
            finish(b, c, buf, 0, &ws);
        },
    )?);

    // extern "C" fn run(x: i64, out: *mut u64): out[0] = tag, out[1..] = payload.
    out.push(define(
        module,
        &format!("{p}_run"),
        run,
        &wsig,
        &[outer],
        |b, refs, ps| {
            let (x, dst) = (ps[0], ps[1]);
            let tag = if c.style == Style::Buffer {
                let payload = b.ins().iadd_imm_s(dst, 8);
                let call = b.ins().call(refs[0], &[x, payload]);
                b.inst_results(call)[0]
            } else {
                let call = b.ins().call(refs[0], &[x]);
                let results = b.inst_results(call).to_vec();
                for (i, w) in results[1..].iter().enumerate() {
                    b.ins()
                        .store(MemFlagsData::trusted(), *w, dst, (8 * (i + 1)) as i32);
                }
                results[0]
            };
            let tag = b.ins().uextend(types::I64, tag);
            b.ins().store(MemFlagsData::trusted(), tag, dst, 0);
            b.ins().return_(&[]);
        },
    )?);

    // fn probe() returns PROBE_TAG and probe_word(i) in every position, so that the listing
    // shows where each position travels.
    out.push(define(
        module,
        &format!("{p}_probe"),
        probe,
        &sig,
        &[],
        |b, _, ps| {
            let ws: Vec<Value> = (0..n)
                .map(|i| b.ins().iconst(types::I64, probe_word(i)))
                .collect();
            finish(b, c, ps.get(1).copied(), PROBE_TAG, &ws);
        },
    )?);
    Ok(out)
}

/// The infallible functions `!` is compared against: `plain_leaf(x) = 2x` and
/// `plain_forward(x) = plain_leaf(x) + 1`, with a `_run` wrapper for `plain_forward` that
/// stores a zero tag.
pub fn build_plain(module: &mut ObjectModule) -> Result<Vec<CompiledFn>, String> {
    let wsig = wrapper_sig(module);
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let leaf = declare(module, "plain_leaf", &sig)?;
    let forward = declare(module, "plain_forward", &sig)?;
    let run = declare(module, "plain_forward_run", &wsig)?;
    let mut out = Vec::new();
    out.push(define(
        module,
        "plain_leaf",
        leaf,
        &sig,
        &[],
        |b, _, ps| {
            let r = b.ins().iadd(ps[0], ps[0]);
            b.ins().return_(&[r]);
        },
    )?);
    out.push(define(
        module,
        "plain_forward",
        forward,
        &sig,
        &[leaf],
        |b, refs, ps| {
            let call = b.ins().call(refs[0], &[ps[0]]);
            let v = b.inst_results(call)[0];
            let r = b.ins().iadd_imm_s(v, 1);
            b.ins().return_(&[r]);
        },
    )?);
    out.push(define(
        module,
        "plain_forward_run",
        run,
        &wsig,
        &[forward],
        |b, refs, ps| {
            let (x, dst) = (ps[0], ps[1]);
            let call = b.ins().call(refs[0], &[x]);
            let v = b.inst_results(call)[0];
            let zero = b.ins().iconst(types::I64, 0);
            b.ins().store(MemFlagsData::trusted(), zero, dst, 0);
            b.ins().store(MemFlagsData::trusted(), v, dst, 8);
            b.ins().return_(&[]);
        },
    )?);
    Ok(out)
}

/// `<prefix>_forward(x) = { val v = <prefix>_leaf(x)!; return v + 1 }`, which differs from
/// `plain_forward` only by `!`, with its `_run` wrapper. Needs the chain `c` (a one-word
/// register chain) in the same module.
pub fn build_forward(module: &mut ObjectModule, c: Chain) -> Result<Vec<CompiledFn>, String> {
    let sig = chain_sig(module, c);
    let wsig = wrapper_sig(module);
    let p = c.prefix;
    let leaf = declare(module, &format!("{p}_leaf"), &sig)?;
    let forward = declare(module, &format!("{p}_forward"), &sig)?;
    let run = declare(module, &format!("{p}_forward_run"), &wsig)?;
    let mut out = Vec::new();
    out.push(define(
        module,
        &format!("{p}_forward"),
        forward,
        &sig,
        &[leaf],
        |b, refs, ps| {
            let v = call_and_propagate(b, c, refs[0], ps[0], None);
            let r = b.ins().iadd_imm_s(v[0], 1);
            finish(b, c, None, 0, &[r]);
        },
    )?);
    out.push(define(
        module,
        &format!("{p}_forward_run"),
        run,
        &wsig,
        &[forward],
        |b, refs, ps| {
            let (x, dst) = (ps[0], ps[1]);
            let call = b.ins().call(refs[0], &[x]);
            let results = b.inst_results(call).to_vec();
            let tag = b.ins().uextend(types::I64, results[0]);
            b.ins().store(MemFlagsData::trusted(), tag, dst, 0);
            b.ins().store(MemFlagsData::trusted(), results[1], dst, 8);
            b.ins().return_(&[]);
        },
    )?);
    Ok(out)
}

/// What Cranelift does with an explicit `StructReturn` parameter beside a tag returned in a
/// register: `Ok` if it compiles, or the error or panic message.
pub fn struct_return_with_tag(triple: Option<&str>) -> Result<(), String> {
    let isa = isa(triple, "speed", false)?;
    let mut module = new_module(isa, "sret")?;
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::special(
        module.target_config().pointer_type(),
        ArgumentPurpose::StructReturn,
    ));
    sig.returns.push(AbiParam::new(types::I32));
    let id = declare(&mut module, "sret_leaf", &sig)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        define(&mut module, "sret_leaf", id, &sig, &[], |b, _, ps| {
            b.ins().store(MemFlagsData::trusted(), ps[0], ps[1], 0);
            let tag = b.ins().iconst(types::I32, 0);
            b.ins().return_(&[tag]);
        })
    }));
    std::panic::set_hook(previous);
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(payload) => Err(payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "a panic without a message".to_string())),
    }
}

/// Where a probe's listing puts the constant `value`: the register it is materialised into
/// (as its 64-bit name), or the memory operand it is stored to, directly or through the next
/// instruction that stores that register. `None` if the listing does not hold the constant.
pub fn placement(listing: &str, value: i64) -> Option<String> {
    let x64 = format!("${value:#x},");
    let arm = format!("#{value}");
    let lines: Vec<&str> = listing.lines().map(str::trim).collect();
    for (k, line) in lines.iter().enumerate() {
        let dst = if let Some(at) = line.find(&x64) {
            // AT&T syntax: `movl $0x2201, %edx` or `movq $0x2202, 8(%rdi)`.
            line[at + x64.len()..].trim().to_string()
        } else {
            let mut tokens = line.split([' ', ',']).filter(|t| !t.is_empty());
            let mnemonic = tokens.next().unwrap_or("");
            let operands: Vec<&str> = tokens.collect();
            if !(mnemonic.starts_with("mov") && operands.get(1) == Some(&arm.as_str())) {
                continue;
            }
            // AArch64: `movz x1, #8705` or `movz w0, #17`.
            operands[0].to_string()
        };
        let reg = wide_register(&dst);
        return Some(stored_to(&lines[k + 1..], &reg).unwrap_or(reg));
    }
    None
}

/// The memory operand the first of `lines` that stores register `reg` writes, if one does:
/// `movq %rsi, (%rdi)` on x86-64, `str x4, [x1, #8]` on AArch64.
fn stored_to(lines: &[&str], reg: &str) -> Option<String> {
    lines.iter().find_map(|l| {
        let (mnemonic, rest) = l.split_once(' ')?;
        let rest = rest.trim();
        if mnemonic == "movq" {
            let (src, dst) = rest.split_once(", ")?;
            (wide_register(src) == reg && dst.contains('(')).then(|| dst.to_string())
        } else if mnemonic == "str" {
            let (src, dst) = rest.split_once(", ")?;
            (wide_register(src) == reg && dst.starts_with('[')).then(|| dst.to_string())
        } else {
            None
        }
    })
}

/// The 64-bit name of an x86-64 or AArch64 register written in a narrower form.
fn wide_register(r: &str) -> String {
    let named = match r {
        "%al" | "%eax" => "%rax",
        "%cl" | "%ecx" => "%rcx",
        "%dl" | "%edx" => "%rdx",
        "%bl" | "%ebx" => "%rbx",
        "%sil" | "%esi" => "%rsi",
        "%dil" | "%edi" => "%rdi",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    if let Some(n) = r
        .strip_prefix("%r")
        .and_then(|s| s.strip_suffix(['d', 'w', 'b']))
    {
        if n.chars().all(|c| c.is_ascii_digit()) {
            return format!("%r{n}");
        }
    }
    if let Some(n) = r.strip_prefix('w') {
        if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
            return format!("x{n}");
        }
    }
    r.to_string()
}

/// Where a chain's probe returns its tag and each payload word, on the target `built` was
/// compiled for: registers, or memory operands for words that travel through memory.
pub fn probe_placements(built: &Built, c: Chain) -> Result<Vec<String>, String> {
    let name = format!("{}_probe", c.prefix);
    let f = built
        .function(&name)
        .ok_or_else(|| format!("no function {name}"))?;
    let mut values = vec![PROBE_TAG];
    values.extend((0..c.words).map(probe_word));
    values
        .iter()
        .map(|v| {
            placement(&f.disasm, *v)
                .ok_or_else(|| format!("{name} does not hold {v:#x}:\n{}", f.disasm))
        })
        .collect()
}

/// The report the spike prints for an object: refused chains, where each chain's probe puts
/// every position, then every function's size, convention, unwind information and listing.
pub fn report(title: &str, built: &Built, chains: &[Chain]) -> String {
    let mut s = format!("# {title}\n");
    for (c, e) in &built.refused {
        let _ = writeln!(s, "refused: {} ({} payload words): {e}", c.prefix, c.words);
    }
    for c in chains {
        if built.refused.iter().any(|(r, _)| r == c) {
            continue;
        }
        match probe_placements(built, *c) {
            Ok(p) => {
                let _ = writeln!(
                    s,
                    "placement: {} = tag {}, payload {}",
                    c.prefix,
                    p[0],
                    p[1..].join(" ")
                );
            }
            Err(e) => {
                let _ = writeln!(s, "placement: {}: {e}", c.prefix);
            }
        }
    }
    for f in &built.functions {
        let _ = writeln!(
            s,
            "\n== {}: {} bytes, {}, unwind {} ==\n{}",
            f.name, f.size, f.call_conv, f.unwind, f.disasm
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const X64_LINUX: &str = TARGETS[0];
    const ARM64_LINUX: &str = TARGETS[1];
    const X64_WINDOWS: &str = TARGETS[2];

    fn build(triple: &str, opt: &str, implicit: bool, chains: &[Chain], extras: bool) -> Built {
        let isa = isa(Some(triple), opt, implicit).unwrap_or_else(|e| panic!("{e}"));
        build_object(isa, "test", chains, extras).unwrap_or_else(|e| panic!("{e}"))
    }

    fn placements(built: &Built, c: Chain) -> Vec<String> {
        probe_placements(built, c).unwrap_or_else(|e| panic!("{e}"))
    }

    fn function<'a>(built: &'a Built, name: &str) -> &'a CompiledFn {
        built
            .function(name)
            .unwrap_or_else(|| panic!("no function {name}"))
    }

    fn refusal(built: &Built, c: Chain) -> Option<&str> {
        built
            .refused
            .iter()
            .find(|(r, _)| *r == c)
            .map(|(_, e)| e.as_str())
    }

    #[test]
    fn test_placement_reads_both_listing_syntaxes() {
        assert_eq!(
            placement("  movl $0x2201, %edx", 0x2201).as_deref(),
            Some("%rdx")
        );
        assert_eq!(
            placement("  movl $0x11, %eax", 0x11).as_deref(),
            Some("%rax")
        );
        assert_eq!(
            placement("  movl $0x2203, %r8d", 0x2203).as_deref(),
            Some("%r8")
        );
        assert_eq!(
            placement("  movq $0x2203, 8(%rdi)", 0x2203).as_deref(),
            Some("8(%rdi)")
        );
        assert_eq!(placement("  movz w0, #17", 0x11).as_deref(), Some("x0"));
        assert_eq!(placement("  movz x1, #8705", 0x2201).as_deref(), Some("x1"));
        assert_eq!(placement("  movz x1, #87050", 0x2201), None);
        assert_eq!(
            placement("  movl $0x2202, %esi\n  movq %rsi, (%rdi)", 0x2202).as_deref(),
            Some("(%rdi)")
        );
        assert_eq!(
            placement("  movz x5, #8706\n  str x5, [x1, #8]", 0x2202).as_deref(),
            Some("[x1, #8]")
        );
    }

    #[test]
    fn test_one_word_payload_returns_in_two_registers_on_every_target() {
        for opt in OPT_LEVELS {
            for (t, regs) in [
                (X64_LINUX, ["%rax", "%rdx"]),
                (X64_WINDOWS, ["%rax", "%rdx"]),
                (ARM64_LINUX, ["x0", "x1"]),
            ] {
                let built = build(t, opt, false, &[PAIR, PAIR32], false);
                assert!(built.refused.is_empty(), "{t}: {:?}", built.refused);
                assert_eq!(placements(&built, PAIR), regs, "{t} {opt}");
                assert_eq!(placements(&built, PAIR32), regs, "{t} {opt}");
            }
        }
    }

    #[test]
    fn test_wider_payload_is_refused_by_x86_64_platform_conventions() {
        for t in [X64_LINUX, X64_WINDOWS] {
            let built = build(t, "speed", false, &[WIDE_PLATFORM, BIG_PLATFORM], false);
            for c in [WIDE_PLATFORM, BIG_PLATFORM] {
                let e = refusal(&built, c).unwrap_or_else(|| panic!("{t} accepted {c:?}"));
                assert!(
                    e.contains("Too many return values to fit in registers"),
                    "{t}: {e}"
                );
            }
        }
        let built = build(
            ARM64_LINUX,
            "speed",
            false,
            &[WIDE_PLATFORM, BIG_PLATFORM],
            false,
        );
        assert!(built.refused.is_empty(), "{:?}", built.refused);
        assert_eq!(placements(&built, WIDE_PLATFORM), ["x0", "x1", "x2"]);
        assert_eq!(placements(&built, BIG_PLATFORM), ["x0", "x1", "x2", "x3"]);
    }

    #[test]
    fn test_tail_convention_returns_wider_payloads_in_registers() {
        for opt in OPT_LEVELS {
            for t in [X64_LINUX, X64_WINDOWS] {
                let built = build(t, opt, false, &[WIDE_TAIL, BIG_TAIL], false);
                assert!(built.refused.is_empty(), "{t}: {:?}", built.refused);
                assert_eq!(placements(&built, WIDE_TAIL), ["%rax", "%rcx", "%rdx"]);
                assert_eq!(
                    placements(&built, BIG_TAIL),
                    ["%rax", "%rcx", "%rdx", "%rsi"]
                );
            }
            // On AArch64 the `tail` convention keeps x0 for a return area and x1 for an
            // indirect callee, so its values start at x2.
            let built = build(ARM64_LINUX, opt, false, &[WIDE_TAIL, BIG_TAIL], false);
            assert_eq!(placements(&built, WIDE_TAIL), ["x2", "x3", "x4"]);
            assert_eq!(placements(&built, BIG_TAIL), ["x2", "x3", "x4", "x5"]);
        }
    }

    #[test]
    fn test_implicit_sret_keeps_two_values_in_registers_and_the_rest_in_memory() {
        for t in [X64_LINUX, X64_WINDOWS] {
            let built = build(t, "speed", true, &IMPLICIT_CHAINS, false);
            assert!(built.refused.is_empty(), "{t}: {:?}", built.refused);
            for c in IMPLICIT_CHAINS {
                let p = placements(&built, c);
                assert_eq!(p[..2], ["%rax", "%rdx"], "{t} {c:?}");
                for m in &p[2..] {
                    assert!(m.contains('('), "{t} {c:?}: word in {m}, not memory");
                }
            }
        }
    }

    #[test]
    fn test_buffer_style_returns_the_tag_alone_in_a_register() {
        for (t, reg) in [
            (X64_LINUX, "%rax"),
            (X64_WINDOWS, "%rax"),
            (ARM64_LINUX, "x0"),
        ] {
            let built = build(t, "speed", false, &[WIDE_BUFFER, BIG_BUFFER], false);
            for c in [WIDE_BUFFER, BIG_BUFFER] {
                let p = placements(&built, c);
                assert_eq!(p[0], reg, "{t}");
                for m in &p[1..] {
                    assert!(m.contains('(') || m.contains('['), "{t}: word in {m}");
                }
            }
        }
    }

    #[test]
    fn test_struct_return_cannot_carry_a_returned_tag() {
        for t in TARGETS {
            let e = struct_return_with_tag(Some(t))
                .err()
                .unwrap_or_else(|| panic!("{t} compiled a StructReturn beside a return value"));
            assert!(
                e.contains("No return values are allowed when using StructReturn"),
                "{t}: {e}"
            );
        }
    }

    #[test]
    fn test_every_function_gets_unwind_information() {
        for (t, kind) in [
            (X64_LINUX, "System V CFI"),
            (ARM64_LINUX, "System V CFI"),
            (X64_WINDOWS, "Windows x64"),
        ] {
            let built = build(t, "speed", false, &MAIN_CHAINS, true);
            for f in &built.functions {
                // A leaf function without a frame needs none on AArch64.
                assert!(
                    f.unwind == kind || (f.unwind == "none" && t == ARM64_LINUX),
                    "{t} {}: {}",
                    f.name,
                    f.unwind
                );
            }
        }
    }

    /// The instructions of the block a function's first conditional branch after its call
    /// jumps to when the tag is not zero, and the instructions between the call and that
    /// branch.
    fn after_call(f: &CompiledFn) -> (Vec<String>, Vec<String>) {
        let lines: Vec<&str> = f.disasm.lines().map(str::trim).collect();
        let call = lines
            .iter()
            .position(|l| l.starts_with("call") || l.starts_with("bl "))
            .unwrap_or_else(|| panic!("{}: no call\n{}", f.name, f.disasm));
        let branch = (call + 1..lines.len())
            .find(|&k| {
                let l = lines[k];
                l.starts_with('j') || l.starts_with("b.") || l.starts_with("cbnz")
            })
            .unwrap_or_else(|| panic!("{}: no branch after the call", f.name));
        // `jnz label4; j label1`, `b.ne label4 ; b label1`, `cbnz w0, label4 ; b label1`.
        let target = lines[branch]
            .split(|c: char| c == ';' || c.is_whitespace() || c == ',')
            .find(|t| t.starts_with("label"))
            .unwrap_or_else(|| panic!("{}: no label in {}", f.name, lines[branch]))
            .replace("label", "block");
        let start = lines
            .iter()
            .position(|l| *l == format!("{target}:"))
            .unwrap_or_else(|| panic!("{}: no {target}", f.name));
        let block: Vec<String> = lines[start + 1..]
            .iter()
            .take_while(|l| !l.starts_with("block"))
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        let between = lines[call + 1..=branch]
            .iter()
            .map(|l| l.to_string())
            .collect();
        (between, block)
    }

    #[test]
    fn test_propagation_is_a_test_and_a_branch_to_the_epilogue() {
        let epilogue = |l: &str| {
            [
                "movq %rbp, %rsp",
                "popq %rbp",
                "retq",
                "addq $0x20, %rsp",
                "ldp fp, lr, [sp], #16",
                "ret",
            ]
            .contains(&l)
        };
        for (t, c, expected) in [
            (X64_LINUX, PAIR, vec!["testb %al, %al", "jnz"]),
            (X64_LINUX, PAIR32, vec!["testl %eax, %eax", "jnz"]),
            (X64_WINDOWS, PAIR, vec!["testb %al, %al", "jnz"]),
            (X64_WINDOWS, PAIR32, vec!["testl %eax, %eax", "jnz"]),
            (ARM64_LINUX, PAIR, vec!["ands wzr, w0, #255", "b.ne"]),
            (ARM64_LINUX, PAIR32, vec!["cbnz w0,"]),
        ] {
            let built = build(t, "speed", false, &[PAIR, PAIR32], true);
            let f = function(&built, &format!("{}_forward", c.prefix));
            let (between, block) = after_call(f);
            assert_eq!(between.len(), expected.len(), "{t} {}: {between:?}", f.name);
            for (got, want) in between.iter().zip(&expected) {
                assert!(got.starts_with(want), "{t} {}: {got} is not {want}", f.name);
            }
            assert!(
                block.iter().all(|l| epilogue(l)),
                "{t} {}: propagating does more than return: {block:?}",
                f.name
            );
        }
    }

    /// The machine instructions of a listing: not block labels, unwind notes or `uninit`
    /// markers, which emit nothing.
    fn instructions(listing: &str) -> usize {
        listing
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with("block")
                    && !l.starts_with("unwind")
                    && !l.starts_with("uninit")
            })
            .count()
    }

    #[test]
    fn test_success_path_adds_only_the_tag_test_and_the_tag() {
        // `<prefix>_forward` and `plain_forward` differ only by `!`. Its success path is
        // every instruction but the propagating block's, and adds to the plain call the tag's
        // test and branch, and the instruction that sets this function's own tag: three on
        // x86-64 (`test`, `jnz`, `xor`), and on AArch64 two with an `i32` tag (`cbnz`, `movz`)
        // and three with an `i8` tag, which is masked first (`ands`, `b.ne`, `movz`).
        for (t, c, added) in [
            (X64_LINUX, PAIR, 3),
            (X64_LINUX, PAIR32, 3),
            (X64_WINDOWS, PAIR, 3),
            (X64_WINDOWS, PAIR32, 3),
            (ARM64_LINUX, PAIR, 3),
            (ARM64_LINUX, PAIR32, 2),
        ] {
            let built = build(t, "speed", false, &[PAIR, PAIR32], true);
            let plain = instructions(&function(&built, "plain_forward").disasm);
            let f = function(&built, &format!("{}_forward", c.prefix));
            let (_, propagate) = after_call(f);
            let success = instructions(&f.disasm) - propagate.len();
            assert_eq!(success, plain + added, "{t} {}:\n{}", f.name, f.disasm);
        }
    }
}

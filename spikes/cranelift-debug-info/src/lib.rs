//! M0b spike: debug and unwind information through Cranelift (README.md).
//!
//! `demo.mn` is compiled by hand into Cranelift IR whose instructions carry their source lines.
//! Cranelift supplies the machine code, the source location of every instruction range, the
//! symbols (through `cranelift-object`), and per-function unwind information. This module adds
//! what Cranelift does not write:
//!
//! - on ELF, the DWARF line table and subprograms (`.debug_info`, `.debug_line`, ...), with
//!   relocations, beside the `.eh_frame` that `cranelift-object` emits from the unwind info;
//! - on COFF, CodeView symbols and line tables (`.debug$S`), which the linker turns into a PDB,
//!   and `.pdata` and `.xdata` from Cranelift's Windows x64 unwind info.

use std::collections::HashMap;

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, SourceLoc, UserFuncName};
use cranelift_codegen::isa::unwind::UnwindInfo;
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use gimli::write::{
    Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Range, RangeList,
    RelocateWriter, Relocation, RelocationTarget, Sections,
};
use gimli::{Encoding, Format, LineEncoding, SectionId};
use object::write::{Object, Relocation as ObjRelocation, SectionId as ObjSection, SymbolId};
use object::{BinaryFormat, RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};

/// What the spike records about one compiled function.
#[derive(Clone, Debug)]
pub struct CompiledFn {
    pub name: String,
    pub id: FuncId,
    pub decl_line: u32,
    pub size: u32,
    /// Code ranges and their source lines, offsets relative to the function's start.
    pub lines: Vec<(u32, u32, u32)>,
    pub unwind: Option<UnwindInfo>,
}

/// The line of `demo.mn` that contains `needle`, 1-based.
pub fn line_of(src: &str, needle: &str) -> Result<u32, String> {
    src.lines()
        .position(|l| l.contains(needle))
        .map(|k| k as u32 + 1)
        .ok_or_else(|| format!("demo.mn has no line containing {needle:?}"))
}

/// The target ISA of the machine running the spike, with unoptimised, position-independent code.
pub fn host_isa() -> Result<OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "none").map_err(|e| e.to_string())?;
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    let builder = cranelift_native::builder().map_err(|e| e.to_string())?;
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}

/// Compiles `demo.mn`'s three functions into an object for the host, with debug and unwind
/// information; `source` is demo.mn's text and `path` the path the debug information names.
pub fn build_object(
    source: &str,
    path: &str,
    comp_dir: &str,
) -> Result<(Vec<u8>, Vec<CompiledFn>), String> {
    let isa = host_isa()?;
    let mut builder = ObjectBuilder::new(isa, "demo", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    builder.unwind_info(true);
    let mut module = ObjectModule::new(builder);
    let call_conv = module.isa().default_call_conv();

    let mut sig1 = module.make_signature();
    sig1.params.push(AbiParam::new(types::I64));
    sig1.returns.push(AbiParam::new(types::I64));
    let mut sig0 = module.make_signature();
    sig0.returns.push(AbiParam::new(types::I64));
    let declare = |m: &mut ObjectModule, name: &str, sig: &cranelift_codegen::ir::Signature| {
        m.declare_function(name, Linkage::Export, sig)
            .map_err(|e| e.to_string())
    };
    let square = declare(&mut module, "square", &sig1)?;
    let sum_squares = declare(&mut module, "sum_squares", &sig1)?;
    let mente_main = declare(&mut module, "mente_main", &sig0)?;

    let l = |needle: &str| line_of(source, needle);
    let mut compiled = Vec::new();
    let mut fctx = FunctionBuilderContext::new();
    let frontend = module.isa().frontend_config();

    // fn square(x: i64) -> i64 { return x * x }
    {
        let mut ctx = Context::new();
        ctx.func.signature = sig1.clone();
        ctx.func.name = UserFuncName::user(0, square.as_u32());
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        b.set_srcloc(SourceLoc::new(l("return x * x")?));
        let x = b.block_params(entry)[0];
        let r = b.ins().imul(x, x);
        b.ins().return_(&[r]);
        b.finalize(frontend);
        compiled.push(define(
            &mut module,
            &mut ctx,
            square,
            "square",
            l("fn square")?,
        )?);
    }

    // fn sum_squares(n: i64) -> i64 { var s = 0; var i = 1; while i <= n { s += square(i); i += 1 }; return s }
    {
        let mut ctx = Context::new();
        ctx.func.signature = sig1.clone();
        ctx.func.name = UserFuncName::user(0, sum_squares.as_u32());
        let callee = module.declare_func_in_func(square, &mut ctx.func);
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let (entry, head, body, exit) = (
            b.create_block(),
            b.create_block(),
            b.create_block(),
            b.create_block(),
        );
        let s = b.declare_var(types::I64);
        let i = b.declare_var(types::I64);
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let n = b.block_params(entry)[0];
        b.set_srcloc(SourceLoc::new(l("var s: i64 = 0")?));
        let zero = b.ins().iconst(types::I64, 0);
        b.def_var(s, zero);
        b.set_srcloc(SourceLoc::new(l("var i: i64 = 1")?));
        let one = b.ins().iconst(types::I64, 1);
        b.def_var(i, one);
        b.ins().jump(head, &[]);
        b.switch_to_block(head);
        b.set_srcloc(SourceLoc::new(l("while i <= n")?));
        let iv = b.use_var(i);
        let c = b.ins().icmp(
            cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual,
            iv,
            n,
        );
        b.ins().brif(c, body, &[], exit, &[]);
        b.switch_to_block(body);
        b.seal_block(body);
        b.set_srcloc(SourceLoc::new(l("s += square(i)")?));
        let iv = b.use_var(i);
        let call = b.ins().call(callee, &[iv]);
        let sq = b.inst_results(call)[0];
        let sv = b.use_var(s);
        let s2 = b.ins().iadd(sv, sq);
        b.def_var(s, s2);
        b.set_srcloc(SourceLoc::new(l("i += 1")?));
        let iv = b.use_var(i);
        let one = b.ins().iconst(types::I64, 1);
        let i2 = b.ins().iadd(iv, one);
        b.def_var(i, i2);
        b.ins().jump(head, &[]);
        b.seal_block(head);
        b.switch_to_block(exit);
        b.seal_block(exit);
        b.set_srcloc(SourceLoc::new(l("return s")?));
        let sv = b.use_var(s);
        b.ins().return_(&[sv]);
        b.finalize(frontend);
        compiled.push(define(
            &mut module,
            &mut ctx,
            sum_squares,
            "sum_squares",
            l("fn sum_squares")?,
        )?);
    }

    // fn mente_main() -> i64 { return sum_squares(10) }
    {
        let mut ctx = Context::new();
        ctx.func.signature = sig0.clone();
        ctx.func.name = UserFuncName::user(0, mente_main.as_u32());
        let callee = module.declare_func_in_func(sum_squares, &mut ctx.func);
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.switch_to_block(entry);
        b.seal_block(entry);
        b.set_srcloc(SourceLoc::new(l("return sum_squares(10)")?));
        let ten = b.ins().iconst(types::I64, 10);
        let call = b.ins().call(callee, &[ten]);
        let r = b.inst_results(call)[0];
        b.ins().return_(&[r]);
        b.finalize(frontend);
        compiled.push(define(
            &mut module,
            &mut ctx,
            mente_main,
            "mente_main",
            l("fn mente_main")?,
        )?);
    }

    let _ = call_conv;
    let mut product = module.finish();
    let symbols: Vec<SymbolId> = compiled
        .iter()
        .map(|f| product.function_symbol(f.id))
        .collect();
    match product.object.format() {
        BinaryFormat::Elf => add_dwarf(&mut product.object, &compiled, &symbols, path, comp_dir)?,
        BinaryFormat::Coff => {
            add_windows_unwind(&mut product.object, &compiled, &symbols)?;
            add_codeview(&mut product.object, &compiled, &symbols, path)?;
        }
        other => return Err(format!("the spike covers ELF and COFF, not {other:?}")),
    }
    let bytes = product.object.write().map_err(|e| e.to_string())?;
    Ok((bytes, compiled))
}

fn define(
    module: &mut ObjectModule,
    ctx: &mut Context,
    id: FuncId,
    name: &str,
    decl_line: u32,
) -> Result<CompiledFn, String> {
    module
        .define_function(id, ctx)
        .map_err(|e| format!("{name}: {e:?}"))?;
    let base = ctx.func.params.base_srcloc();
    let code = ctx.compiled_code().ok_or("no compiled code")?;
    let size = code.buffer.data().len() as u32;
    // Instructions Cranelift adds itself (the prologue and epilogue) carry no source location;
    // they are attributed to the function's declaration line.
    let lines = code
        .buffer
        .get_srclocs_sorted()
        .iter()
        .map(|s| {
            let loc = if s.loc.is_abs() {
                s.loc.as_abs()
            } else {
                s.loc.relocate(base)
            };
            (
                s.start,
                s.end,
                if loc.is_default() {
                    decl_line
                } else {
                    loc.bits()
                },
            )
        })
        .collect();
    let unwind = code
        .create_unwind_info(module.isa())
        .map_err(|e| e.to_string())?;
    Ok(CompiledFn {
        name: name.to_string(),
        id,
        decl_line,
        size,
        lines,
        unwind,
    })
}

/// The rows of a line table: every code range in order, with a row at the start of the function.
fn line_rows(f: &CompiledFn) -> Vec<(u32, u32)> {
    let mut rows: Vec<(u32, u32)> = Vec::new();
    if f.lines.first().is_none_or(|(start, _, _)| *start > 0) {
        rows.push((0, f.decl_line));
    }
    for &(start, _, line) in &f.lines {
        if rows.last().is_none_or(|(_, l)| *l != line) {
            rows.push((start, line));
        }
    }
    rows
}

// ---------------------------------------------------------------- DWARF on ELF

#[derive(Clone)]
struct RelocSection {
    writer: EndianVec<gimli::RunTimeEndian>,
    relocations: Vec<Relocation>,
}

impl RelocateWriter for RelocSection {
    type Writer = EndianVec<gimli::RunTimeEndian>;
    fn writer(&self) -> &Self::Writer {
        &self.writer
    }
    fn writer_mut(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }
    fn relocate(&mut self, relocation: Relocation) {
        self.relocations.push(relocation);
    }
}

fn add_dwarf(
    obj: &mut Object<'static>,
    fns: &[CompiledFn],
    symbols: &[SymbolId],
    path: &str,
    comp_dir: &str,
) -> Result<(), String> {
    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(comp_dir.as_bytes().to_vec()),
        None,
        LineString::String(path.as_bytes().to_vec()),
        None,
    );
    let dir = program.default_directory();
    let file = program.add_file(LineString::String(path.as_bytes().to_vec()), dir, None);
    for (k, f) in fns.iter().enumerate() {
        program.begin_sequence(Some(Address::Symbol {
            symbol: k,
            addend: 0,
        }));
        for (offset, line) in line_rows(f) {
            let row = program.row();
            row.address_offset = u64::from(offset);
            row.file = file;
            row.line = u64::from(line);
            row.is_statement = true;
            program.generate_row();
        }
        program.end_sequence(u64::from(f.size));
    }
    dwarf.unit.line_program = program;

    let ranges = RangeList(
        fns.iter()
            .enumerate()
            .map(|(k, f)| Range::StartLength {
                begin: Address::Symbol {
                    symbol: k,
                    addend: 0,
                },
                length: u64::from(f.size),
            })
            .collect(),
    );
    let range_id = dwarf.unit.ranges.add(ranges);
    let root = dwarf.unit.root();
    let cu = dwarf.unit.get_mut(root);
    cu.set(
        gimli::DW_AT_producer,
        AttributeValue::String(b"mente spike (Cranelift 0.137)".to_vec()),
    );
    // Mente has no DWARF language code yet; C keeps debuggers' expression evaluators usable.
    cu.set(
        gimli::DW_AT_language,
        AttributeValue::Language(gimli::DW_LANG_C11),
    );
    cu.set(
        gimli::DW_AT_name,
        AttributeValue::String(path.as_bytes().to_vec()),
    );
    cu.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(comp_dir.as_bytes().to_vec()),
    );
    cu.set(
        gimli::DW_AT_low_pc,
        AttributeValue::Address(Address::Constant(0)),
    );
    cu.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(range_id));
    for (k, f) in fns.iter().enumerate() {
        let sp = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
        let e = dwarf.unit.get_mut(sp);
        e.set(
            gimli::DW_AT_name,
            AttributeValue::String(f.name.as_bytes().to_vec()),
        );
        e.set(gimli::DW_AT_external, AttributeValue::Flag(true));
        e.set(
            gimli::DW_AT_decl_file,
            AttributeValue::FileIndex(Some(file)),
        );
        e.set(
            gimli::DW_AT_decl_line,
            AttributeValue::Udata(u64::from(f.decl_line)),
        );
        e.set(
            gimli::DW_AT_low_pc,
            AttributeValue::Address(Address::Symbol {
                symbol: k,
                addend: 0,
            }),
        );
        e.set(
            gimli::DW_AT_high_pc,
            AttributeValue::Udata(u64::from(f.size)),
        );
        e.set(
            gimli::DW_AT_frame_base,
            AttributeValue::Exprloc(gimli::write::Expression::raw(vec![
                gimli::DW_OP_call_frame_cfa.0,
            ])),
        );
    }

    // Both of the spike's targets, x86-64 and AArch64, are little-endian.
    let endian = gimli::RunTimeEndian::Little;
    let mut sections = Sections::new(RelocSection {
        writer: EndianVec::new(endian),
        relocations: Vec::new(),
    });
    dwarf.write(&mut sections).map_err(|e| e.to_string())?;

    let mut ids: HashMap<SectionId, ObjSection> = HashMap::new();
    sections.for_each(|id, s| -> Result<(), String> {
        if !s.writer.slice().is_empty() {
            let section = obj.add_section(
                Vec::new(),
                id.name().as_bytes().to_vec(),
                SectionKind::Debug,
            );
            obj.set_section_data(section, s.writer.slice().to_vec(), 1);
            ids.insert(id, section);
        }
        Ok(())
    })?;
    sections.for_each(|id, s| -> Result<(), String> {
        let Some(&section) = ids.get(&id) else {
            return Ok(());
        };
        for r in &s.relocations {
            let (symbol, addend) = match r.target {
                RelocationTarget::Symbol(k) => (symbols[k], r.addend),
                RelocationTarget::Section(target) => {
                    let t = *ids.get(&target).ok_or(format!(
                        "{} refers to an empty {}",
                        id.name(),
                        target.name()
                    ))?;
                    (obj.section_symbol(t), r.addend)
                }
            };
            let flags = RelocationFlags::Generic {
                kind: RelocationKind::Absolute,
                encoding: RelocationEncoding::Generic,
                size: r.size * 8,
            };
            obj.add_relocation(
                section,
                ObjRelocation {
                    offset: r.offset as u64,
                    symbol,
                    addend,
                    flags,
                },
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- Windows: unwind and CodeView

const IMAGE_REL_AMD64_ADDR32NB: u16 = 0x0003;
const IMAGE_REL_AMD64_SECTION: u16 = 0x000A;
const IMAGE_REL_AMD64_SECREL: u16 = 0x000B;

fn coff(typ: u16) -> RelocationFlags {
    RelocationFlags::Coff {
        typ: object::pe::RelocationType(typ),
    }
}

/// `.xdata` holds each function's UNWIND_INFO, as Cranelift encodes it; `.pdata` holds a
/// RUNTIME_FUNCTION per function, whose three image-relative addresses the linker fills in.
fn add_windows_unwind(
    obj: &mut Object<'static>,
    fns: &[CompiledFn],
    symbols: &[SymbolId],
) -> Result<(), String> {
    let xdata = obj.add_section(Vec::new(), b".xdata".to_vec(), SectionKind::ReadOnlyData);
    let pdata = obj.add_section(Vec::new(), b".pdata".to_vec(), SectionKind::ReadOnlyData);
    let mut x = Vec::new();
    let mut p = Vec::new();
    let mut prelocs = Vec::new();
    for (f, &sym) in fns.iter().zip(symbols) {
        let Some(UnwindInfo::WindowsX64(info)) = &f.unwind else {
            return Err(format!(
                "{}: Cranelift gave no Windows x64 unwind information",
                f.name
            ));
        };
        while !x.len().is_multiple_of(4) {
            x.push(0);
        }
        let at = x.len();
        let mut buf = vec![0u8; info.emit_size()];
        info.emit(&mut buf);
        x.extend_from_slice(&buf);
        let entry = p.len() as u64;
        p.extend_from_slice(&[0u8; 12]);
        prelocs.push((entry, sym, 0i64));
        prelocs.push((entry + 4, sym, i64::from(f.size)));
        prelocs.push((entry + 8, obj.section_symbol(xdata), at as i64));
    }
    obj.set_section_data(xdata, x, 4);
    obj.set_section_data(pdata, p, 4);
    for (offset, symbol, addend) in prelocs {
        obj.add_relocation(
            pdata,
            ObjRelocation {
                offset,
                symbol,
                addend,
                flags: coff(IMAGE_REL_AMD64_ADDR32NB),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One CodeView symbol record: its length, kind and payload, padded to four bytes.
fn cv_record(out: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(payload);
    while !(body.len() + 2).is_multiple_of(4) {
        body.push(0);
    }
    out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    out.extend_from_slice(&body);
}

fn cv_subsection(out: &mut Vec<u8>, kind: u32, data: &[u8]) -> usize {
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    let start = out.len();
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    start
}

const S_END: u16 = 0x0006;
const S_OBJNAME: u16 = 0x1101;
const S_GPROC32: u16 = 0x1110;
const S_COMPILE3: u16 = 0x113C;
const DEBUG_S_SYMBOLS: u32 = 0xF1;
const DEBUG_S_LINES: u32 = 0xF2;
const DEBUG_S_STRINGTABLE: u32 = 0xF3;
const DEBUG_S_FILECHKSMS: u32 = 0xF4;

/// `.debug$S`: the object's name and compiler, a procedure per function, the file table, and a
/// line table per function. Each procedure and line table names its function's offset and
/// section through SECREL and SECTION relocations.
fn add_codeview(
    obj: &mut Object<'static>,
    fns: &[CompiledFn],
    symbols: &[SymbolId],
    path: &str,
) -> Result<(), String> {
    let mut relocs: Vec<(u64, SymbolId, u16)> = Vec::new();
    let mut out = 4u32.to_le_bytes().to_vec(); // CV_SIGNATURE_C13

    let mut syms = Vec::new();
    let mut objname = 0u32.to_le_bytes().to_vec();
    objname.extend_from_slice(b"demo.obj\0");
    cv_record(&mut syms, S_OBJNAME, &objname);
    let mut compile = 0u32.to_le_bytes().to_vec(); // language 0 (C), no flags
    compile.extend_from_slice(&0xD0u16.to_le_bytes()); // CV_CFL_AMD64
    for v in [0u16, 1, 0, 0, 0, 137, 1, 0] {
        compile.extend_from_slice(&v.to_le_bytes());
    }
    compile.extend_from_slice(b"mente spike (Cranelift 0.137)\0");
    cv_record(&mut syms, S_COMPILE3, &compile);
    let mut proc_fixups = Vec::new();
    for (f, &sym) in fns.iter().zip(symbols) {
        let mut p = Vec::new();
        for v in [0u32, 0, 0, f.size, 0, f.size, 0] {
            p.extend_from_slice(&v.to_le_bytes()); // parent, end, next, length, debug start, debug end, type
        }
        let off_at = p.len();
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p.push(0); // flags
        p.extend_from_slice(f.name.as_bytes());
        p.push(0);
        let record_at = syms.len() + 4; // past the record's length and kind
        cv_record(&mut syms, S_GPROC32, &p);
        proc_fixups.push((record_at + off_at, sym));
        cv_record(&mut syms, S_END, &[]);
    }
    let syms_at = cv_subsection(&mut out, DEBUG_S_SYMBOLS, &syms);
    for (at, sym) in proc_fixups {
        relocs.push(((syms_at + at) as u64, sym, IMAGE_REL_AMD64_SECREL));
        relocs.push(((syms_at + at + 4) as u64, sym, IMAGE_REL_AMD64_SECTION));
    }

    let mut strings = vec![0u8];
    strings.extend_from_slice(path.as_bytes());
    strings.push(0);
    cv_subsection(&mut out, DEBUG_S_STRINGTABLE, &strings);
    let mut checksums = 1u32.to_le_bytes().to_vec(); // the name's offset in the string table
    checksums.extend_from_slice(&[0, 0, 0, 0]); // no checksum, padding
    cv_subsection(&mut out, DEBUG_S_FILECHKSMS, &checksums);

    for (f, &sym) in fns.iter().zip(symbols) {
        let rows = line_rows(f);
        let mut lines = Vec::new();
        lines.extend_from_slice(&0u32.to_le_bytes()); // offset, relocated
        lines.extend_from_slice(&0u16.to_le_bytes()); // section, relocated
        lines.extend_from_slice(&0u16.to_le_bytes()); // flags
        lines.extend_from_slice(&f.size.to_le_bytes());
        lines.extend_from_slice(&0u32.to_le_bytes()); // file: offset 0 in the checksum table
        lines.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        lines.extend_from_slice(&(12 + 8 * rows.len() as u32).to_le_bytes());
        for (offset, line) in rows {
            lines.extend_from_slice(&offset.to_le_bytes());
            lines.extend_from_slice(&((line & 0x00FF_FFFF) | 0x8000_0000).to_le_bytes());
            // a statement
        }
        let at = cv_subsection(&mut out, DEBUG_S_LINES, &lines);
        relocs.push((at as u64, sym, IMAGE_REL_AMD64_SECREL));
        relocs.push((at as u64 + 4, sym, IMAGE_REL_AMD64_SECTION));
    }

    let section = obj.add_section(Vec::new(), b".debug$S".to_vec(), SectionKind::Debug);
    obj.set_section_data(section, out, 4);
    for (offset, symbol, typ) in relocs {
        obj.add_relocation(
            section,
            ObjRelocation {
                offset,
                symbol,
                addend: 0,
                flags: coff(typ),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::{Object as _, ObjectSection as _};
    use std::collections::BTreeSet;

    const SOURCE: &str = include_str!("../demo.mn");

    fn expected_lines(name: &str) -> BTreeSet<u32> {
        let l = |n: &str| line_of(SOURCE, n).unwrap();
        match name {
            "square" => [l("fn square"), l("return x * x")].into(),
            "sum_squares" => [
                l("fn sum_squares"),
                l("var s: i64 = 0"),
                l("var i: i64 = 1"),
                l("while i <= n"),
                l("s += square(i)"),
                l("i += 1"),
                l("return s"),
            ]
            .into(),
            _ => [l("fn mente_main"), l("return sum_squares(10)")].into(),
        }
    }

    #[test]
    fn test_cranelift_gives_every_function_its_lines_and_unwind_information() {
        let (_, fns) = build_object(SOURCE, "demo.mn", "/src").unwrap();
        assert_eq!(fns.len(), 3);
        for f in &fns {
            let lines: BTreeSet<u32> = line_rows(f).into_iter().map(|(_, l)| l).collect();
            assert_eq!(lines, expected_lines(&f.name), "{}", f.name);
            assert!(f.unwind.is_some(), "{}", f.name);
            assert!(
                f.lines.iter().all(|(s, e, _)| s < e && *e <= f.size),
                "{}: {:?}",
                f.name,
                f.lines
            );
        }
    }

    #[test]
    fn test_the_object_carries_its_debug_sections_with_relocations() {
        let (bytes, fns) = build_object(SOURCE, "demo.mn", "/src").unwrap();
        let file = object::File::parse(&*bytes).unwrap();
        let names: Vec<String> = file
            .sections()
            .filter_map(|s| s.name().ok().map(str::to_string))
            .collect();
        let relocs = |name: &str| {
            file.section_by_name(name)
                .map_or(0, |s| s.relocations().count())
        };
        match file.format() {
            BinaryFormat::Elf => {
                for s in [".debug_info", ".debug_line", ".debug_abbrev", ".eh_frame"] {
                    assert!(names.iter().any(|n| n == s), "{s} missing from {names:?}");
                }
                // One address per subprogram and per range, one per line sequence.
                assert!(relocs(".debug_info") >= fns.len(), "{names:?}");
                assert!(relocs(".debug_line") >= fns.len());
                assert!(relocs(".eh_frame") >= fns.len());
                // Read the line program back: each sequence holds its function's lines.
                let load = |id: gimli::SectionId| -> Result<
                    gimli::EndianSlice<'_, gimli::LittleEndian>,
                    gimli::Error,
                > {
                    Ok(gimli::EndianSlice::new(
                        file.section_by_name(id.name())
                            .and_then(|s| s.data().ok())
                            .unwrap_or(&[]),
                        gimli::LittleEndian,
                    ))
                };
                let dwarf = gimli::Dwarf::load(load).unwrap();
                let header = dwarf.units().next().unwrap().unwrap();
                let unit = dwarf.unit(header).unwrap();
                let program = unit.line_program.clone().unwrap();
                let mut rows = program.rows();
                let mut sequences: Vec<BTreeSet<u32>> = vec![BTreeSet::new()];
                while let Some((_, row)) = rows.next_row().unwrap() {
                    if row.end_sequence() {
                        sequences.push(BTreeSet::new());
                    } else if let Some(line) = row.line() {
                        sequences.last_mut().unwrap().insert(line.get() as u32);
                    }
                }
                sequences.pop();
                let wanted: Vec<BTreeSet<u32>> =
                    fns.iter().map(|f| expected_lines(&f.name)).collect();
                assert_eq!(sequences, wanted);
            }
            BinaryFormat::Coff => {
                for s in [".debug$S", ".pdata", ".xdata"] {
                    assert!(names.iter().any(|n| n == s), "{s} missing from {names:?}");
                }
                assert_eq!(relocs(".pdata"), 3 * fns.len());
                // A SECREL and a SECTION relocation per procedure and per line table.
                assert_eq!(relocs(".debug$S"), 4 * fns.len());
                let data = file.section_by_name(".debug$S").unwrap().data().unwrap();
                assert_eq!(&data[..4], &4u32.to_le_bytes());
                for f in &fns {
                    assert!(
                        data.windows(f.name.len() + 1)
                            .any(|w| w == [f.name.as_bytes(), &[0]].concat()),
                        "{}",
                        f.name
                    );
                }
                // Read the line tables back.
                let mut at = 4;
                let mut tables: Vec<BTreeSet<u32>> = Vec::new();
                while at + 8 <= data.len() {
                    let kind = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let len = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
                    let body = &data[at + 8..at + 8 + len];
                    if kind == DEBUG_S_LINES {
                        let n = u32::from_le_bytes(body[16..20].try_into().unwrap()) as usize;
                        tables.push(
                            (0..n)
                                .map(|k| {
                                    u32::from_le_bytes(
                                        body[28 + 8 * k..32 + 8 * k].try_into().unwrap(),
                                    ) & 0x00FF_FFFF
                                })
                                .collect(),
                        );
                    }
                    at += 8 + len.div_ceil(4) * 4;
                }
                let wanted: Vec<BTreeSet<u32>> =
                    fns.iter().map(|f| expected_lines(&f.name)).collect();
                assert_eq!(tables, wanted);
            }
            other => panic!("unexpected format {other:?}"),
        }
    }
}

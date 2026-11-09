//! `mente-codegen` — native code generation. Governed by `spec/mir-and-codegen.md`,
//! `spec/error-handling-abi.md` and `spec/hardware-contracts.md`.
//!
//! Cranelift compiles each function for one of the [`Target`]s, and [`ObjectWriter`] places its
//! machine code and symbol in an object file through `cranelift-object`. Cranelift gives the
//! code range of every source location and each function's unwind information, but writes
//! little of the debug information made from them (`spikes/cranelift-debug-info/README.md`,
//! finding 1), so this crate adds it:
//!
//! - on ELF ([`elf`]), the DWARF compilation unit, its subprograms and its line table, written
//!   with `gimli`, and `.eh_frame` with PC-relative FDE pointers;
//! - on COFF ([`coff`]), `.pdata` and `.xdata` from Cranelift's Windows x64 unwind information,
//!   and the CodeView symbols and line tables of `.debug$S`, which the linker turns into a PDB.
//!
//! Line tables are made from the code ranges Cranelift gives, as they are, never from an assumed
//! layout of the code (finding 4). Nothing here depends on MIR: lowering MIR to Cranelift IR
//! comes next in M1's checkpoint (c) (`ROADMAP.md`). This is the only crate that may depend on
//! Cranelift, `gimli` and `object` (`CLAUDE.md`, invariant 2).

use std::collections::HashMap;
use std::fmt;

use cranelift_codegen::ir::SourceLoc;
use cranelift_codegen::isa::unwind::UnwindInfo;
use cranelift_codegen::isa::{self, OwnedTargetIsa, TargetIsa};
use cranelift_codegen::settings::{self, Configurable, OptLevel};
use cranelift_codegen::{CompiledCode, Context};
use cranelift_module::{FuncId, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use object::write::SymbolId;

mod coff;
mod elf;
pub mod link;
pub mod lower;
pub mod native;
#[cfg(test)]
mod test_program;

/// The version of Cranelift that `Cargo.toml` pins, which the debug information names.
const CRANELIFT_VERSION: &str = "0.137.1";

/// What the debug information names as the producer of the code.
fn producer() -> String {
    format!(
        "mente-codegen {} (Cranelift {CRANELIFT_VERSION})",
        env!("CARGO_PKG_VERSION")
    )
}

/// Why an ISA could not be built, a function compiled or an object written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodegenError(String);

impl CodegenError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodegenError {}

/// The targets Mente writes objects for: the three CI runs and debugs native code on. Any other
/// is refused, rather than written untested.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// `x86_64-unknown-linux-gnu`: ELF, DWARF and `.eh_frame`.
    X86_64Linux,
    /// `aarch64-unknown-linux-gnu`: ELF, DWARF and `.eh_frame`.
    Aarch64Linux,
    /// `x86_64-pc-windows-msvc`: COFF, CodeView, `.pdata` and `.xdata`.
    X86_64Windows,
}

/// Which processor features an ISA may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cpu {
    /// Only the target's baseline, so that the code runs on every processor of the target.
    Baseline,
    /// Every feature of the processor running the compiler, which `cranelift-native` detects;
    /// for code that runs where it is compiled. Only the host's own target has it.
    Host,
}

impl Target {
    /// Every target, in a fixed order.
    pub const ALL: [Target; 3] = [
        Target::X86_64Linux,
        Target::Aarch64Linux,
        Target::X86_64Windows,
    ];

    /// The target triple, as Cranelift and `rustc` spell it.
    pub fn triple(self) -> &'static str {
        match self {
            Target::X86_64Linux => "x86_64-unknown-linux-gnu",
            Target::Aarch64Linux => "aarch64-unknown-linux-gnu",
            Target::X86_64Windows => "x86_64-pc-windows-msvc",
        }
    }

    /// The target of the machine running the compiler, if it is one of the three.
    pub fn host() -> Option<Target> {
        Target::of_system(std::env::consts::ARCH, std::env::consts::OS)
    }

    /// The target of a machine, named by its architecture and operating system as
    /// `std::env::consts` names them.
    fn of_system(arch: &str, os: &str) -> Option<Target> {
        match (arch, os) {
            ("x86_64", "linux") => Some(Target::X86_64Linux),
            ("aarch64", "linux") => Some(Target::Aarch64Linux),
            ("x86_64", "windows") => Some(Target::X86_64Windows),
            _ => None,
        }
    }

    /// The target an ISA generates code for, if it is one of the three.
    fn of_isa(isa: &dyn TargetIsa) -> Option<Target> {
        let triple = isa.triple().to_string();
        Target::ALL.into_iter().find(|t| t.triple() == triple)
    }

    /// A Cranelift ISA for this target that generates position-independent code, optimised at
    /// `opt_level`, for the processor features `cpu` allows.
    pub fn isa(self, opt_level: OptLevel, cpu: Cpu) -> Result<OwnedTargetIsa, CodegenError> {
        let mut flags = settings::builder();
        let setting_error = |e: settings::SetError| CodegenError::new(e.to_string());
        flags
            .set("opt_level", &opt_level.to_string())
            .map_err(setting_error)?;
        flags.set("is_pic", "true").map_err(setting_error)?;
        let mut builder = isa::lookup_by_name(self.triple())
            .map_err(|e| CodegenError::new(format!("{}: {e}", self.triple())))?;
        if cpu == Cpu::Host {
            if Target::host() != Some(self) {
                return Err(CodegenError::new(format!(
                    "the processor running the compiler has no {} features to detect",
                    self.triple()
                )));
            }
            cranelift_native::infer_native_flags(&mut builder).map_err(CodegenError::new)?;
        }
        builder
            .finish(settings::Flags::new(flags))
            .map_err(|e| CodegenError::new(format!("{}: {e}", self.triple())))
    }
}

/// A position in the one `.mn` file a module comes from: a 1-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePos {
    pub line: u32,
    pub column: u32,
}

/// The `.mn` file an object's code comes from, as its debug information names it: debuggers
/// look the file up by `path`, relative to `directory` unless it is absolute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    /// The compilation directory.
    pub directory: String,
}

/// The source positions a writer has handed out as Cranelift `SourceLoc`s: a `SourceLoc`'s bits
/// are the position's index. Every index stays below `limit`, the bits of the default
/// `SourceLoc`, which marks an instruction without a position.
#[derive(Clone, Debug)]
struct Positions {
    list: Vec<SourcePos>,
    index: HashMap<SourcePos, u32>,
    limit: u32,
}

impl Positions {
    fn new(limit: u32) -> Self {
        Positions {
            list: Vec::new(),
            index: HashMap::new(),
            limit,
        }
    }

    fn intern(&mut self, position: SourcePos) -> Result<SourceLoc, CodegenError> {
        if let Some(&k) = self.index.get(&position) {
            return Ok(SourceLoc::new(k));
        }
        let k = u32::try_from(self.list.len())
            .ok()
            .filter(|k| *k < self.limit)
            .ok_or_else(|| {
                CodegenError::new(format!(
                    "a module has at most {} distinct source positions",
                    self.limit
                ))
            })?;
        self.list.push(position);
        self.index.insert(position, k);
        Ok(SourceLoc::new(k))
    }

    /// The position a `SourceLoc` from `intern` stands for.
    fn get(&self, loc: SourceLoc) -> Option<SourcePos> {
        self.list.get(loc.bits() as usize).copied()
    }
}

/// The code of a function from `start` to `end`, offsets from the function's first byte, and
/// the source position it was compiled from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodeRange {
    start: u32,
    end: u32,
    position: SourcePos,
}

/// What a writer records about a function it defined.
#[derive(Clone, Debug)]
struct CompiledFunction {
    id: FuncId,
    decl: SourcePos,
    size: u32,
    ranges: Vec<CodeRange>,
    unwind: Option<UnwindInfo>,
}

/// A defined function as the object holds it, which the debug information describes.
#[derive(Clone, Debug)]
struct Function {
    name: String,
    symbol: SymbolId,
    decl: SourcePos,
    size: u32,
    ranges: Vec<CodeRange>,
    unwind: Option<UnwindInfo>,
}

/// One row of a line table: from `offset` on, the code was compiled from `position`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    offset: u32,
    position: SourcePos,
}

/// A function's line rows: one wherever the source position changes, and the declaration's
/// position from offset 0 when the first code range starts later or there is none.
fn rows(decl: SourcePos, ranges: &[CodeRange]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    if ranges.first().is_none_or(|r| r.start > 0) {
        rows.push(Row {
            offset: 0,
            position: decl,
        });
    }
    for r in ranges {
        if rows.last().is_none_or(|last| last.position != r.position) {
            rows.push(Row {
                offset: r.start,
                position: r.position,
            });
        }
    }
    rows
}

/// The code ranges of a compiled function, with their source positions. Instructions without a
/// position, such as the prologue and epilogue Cranelift adds, belong to the declaration.
fn code_ranges(
    code: &CompiledCode,
    base: SourceLoc,
    decl: SourcePos,
    positions: &Positions,
) -> Result<Vec<CodeRange>, CodegenError> {
    code.buffer
        .get_srclocs_sorted()
        .iter()
        .map(|s| {
            let loc = if s.loc.is_abs() {
                s.loc.as_abs()
            } else {
                s.loc.relocate(base)
            };
            let position = if loc.is_default() {
                decl
            } else {
                positions.get(loc).ok_or_else(|| {
                    CodegenError::new(format!(
                        "source location {loc} was not handed out by ObjectWriter::source_loc"
                    ))
                })?
            };
            Ok(CodeRange {
                start: s.start,
                end: s.end,
                position,
            })
        })
        .collect()
}

/// Writes an object file for one module: the machine code and symbols of its functions, their
/// line tables and their unwind information.
///
/// Declare functions through [`ObjectWriter::module_mut`], give each instruction a `SourceLoc`
/// from [`ObjectWriter::source_loc`], define every function with
/// [`ObjectWriter::define_function`], and then call [`ObjectWriter::finish`].
pub struct ObjectWriter {
    target: Target,
    module: ObjectModule,
    source: SourceFile,
    object_name: String,
    positions: Positions,
    functions: Vec<CompiledFunction>,
}

impl ObjectWriter {
    /// A writer for code `isa` generates, which must be an ISA from [`Target::isa`]. `source`
    /// is the file the code comes from; `object_name` is the name the object file is written
    /// under, which CodeView records.
    pub fn new(
        isa: OwnedTargetIsa,
        source: SourceFile,
        object_name: &str,
    ) -> Result<Self, CodegenError> {
        let target = Target::of_isa(&*isa).ok_or_else(|| {
            CodegenError::new(format!(
                "mente-codegen writes objects for {}, not for {}",
                Target::ALL.map(Target::triple).join(", "),
                isa.triple()
            ))
        })?;
        let mut builder = ObjectBuilder::new(
            isa,
            source.path.as_bytes(),
            cranelift_module::default_libcall_names(),
        )
        .map_err(|e| CodegenError::new(e.to_string()))?;
        // cranelift-object writes `.eh_frame` with absolute FDE pointers, which a
        // position-independent executable cannot link (the spike's finding 2); `elf.rs` writes
        // it with PC-relative ones instead.
        builder.unwind_info(false);
        Ok(ObjectWriter {
            target,
            module: ObjectModule::new(builder),
            source,
            object_name: object_name.to_string(),
            positions: Positions::new(SourceLoc::default().bits()),
            functions: Vec::new(),
        })
    }

    /// The target the object is for.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The module that declares the object's functions and data, and builds their signatures.
    pub fn module(&self) -> &ObjectModule {
        &self.module
    }

    /// The module, to declare functions and data in, and to reference them from a function.
    /// A function defined through it rather than [`ObjectWriter::define_function`] has no line
    /// table or unwind information, and `finish` refuses it.
    pub fn module_mut(&mut self) -> &mut ObjectModule {
        &mut self.module
    }

    /// The Cranelift `SourceLoc` that stands for `position`, for `FunctionBuilder::set_srcloc`.
    pub fn source_loc(&mut self, position: SourcePos) -> Result<SourceLoc, CodegenError> {
        self.positions.intern(position)
    }

    /// Compiles the function in `ctx` for the writer's target, places its code at the symbol
    /// `id` declares, and records its code ranges and unwind information. `decl` is the position
    /// of its declaration, to which instructions without a position belong.
    pub fn define_function(
        &mut self,
        id: FuncId,
        ctx: &mut Context,
        decl: SourcePos,
    ) -> Result<(), CodegenError> {
        self.module
            .define_function(id, ctx)
            .map_err(|e| CodegenError::new(format!("{e:?}")))?;
        let name = self
            .module
            .declarations()
            .get_function_decl(id)
            .linkage_name(id)
            .into_owned();
        let in_function = |message: String| CodegenError::new(format!("{name}: {message}"));
        let code = ctx
            .compiled_code()
            .ok_or_else(|| in_function("Cranelift kept no compiled code".to_string()))?;
        let size = u32::try_from(code.buffer.data().len())
            .map_err(|_| in_function("the code is larger than 4 GiB".to_string()))?;
        let ranges = code_ranges(code, ctx.func.params.base_srcloc(), decl, &self.positions)
            .map_err(|e| in_function(e.0))?;
        let unwind = code
            .create_unwind_info(self.module.isa())
            .map_err(|e| in_function(e.to_string()))?;
        self.functions.push(CompiledFunction {
            id,
            decl,
            size,
            ranges,
            unwind,
        });
        Ok(())
    }

    /// The object file, with the debug and unwind information of every function defined.
    pub fn finish(self) -> Result<Vec<u8>, CodegenError> {
        let cie = self.module.isa().create_systemv_cie();
        let names: HashMap<FuncId, String> = self
            .module
            .declarations()
            .get_functions()
            .map(|(id, decl)| (id, decl.linkage_name(id).into_owned()))
            .collect();
        let mut product = self.module.finish();
        for (id, entry) in product.functions.iter() {
            if matches!(entry, Some((_, true))) && !self.functions.iter().any(|f| f.id == id) {
                return Err(CodegenError::new(format!(
                    "{} was defined without ObjectWriter::define_function, so it has no line \
                     table or unwind information",
                    names.get(&id).map_or("a function", String::as_str)
                )));
            }
        }
        let mut functions = Vec::new();
        for f in self.functions {
            let Some((symbol, true)) = product.functions[f.id] else {
                return Err(CodegenError::new(format!(
                    "function {} has no symbol in the object",
                    f.id
                )));
            };
            functions.push(Function {
                name: String::from_utf8_lossy(&product.object.symbol(symbol).name).into_owned(),
                symbol,
                decl: f.decl,
                size: f.size,
                ranges: f.ranges,
                unwind: f.unwind,
            });
        }
        match self.target {
            Target::X86_64Linux | Target::Aarch64Linux => {
                elf::add_eh_frame(&mut product.object, &functions, cie)?;
                elf::add_dwarf(&mut product.object, &functions, &self.source)?;
            }
            Target::X86_64Windows => {
                coff::add_unwind_info(&mut product.object, &functions)?;
                coff::add_codeview(
                    &mut product.object,
                    &functions,
                    &self.source,
                    &self.object_name,
                )?;
            }
        }
        product
            .object
            .write()
            .map_err(|e| CodegenError::new(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_program::{self, position_of, SOURCE};
    use cranelift_codegen::ir::{types, AbiParam, InstBuilder, UserFuncName};
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
    use cranelift_module::Linkage;
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
    use std::collections::BTreeSet;

    fn pos(line: u32, column: u32) -> SourcePos {
        SourcePos { line, column }
    }

    fn range(start: u32, end: u32, position: SourcePos) -> CodeRange {
        CodeRange {
            start,
            end,
            position,
        }
    }

    #[test]
    fn test_each_target_has_its_isa_and_object_format() {
        for target in Target::ALL {
            let isa = target.isa(OptLevel::Speed, Cpu::Baseline).unwrap();
            assert_eq!(isa.triple().to_string(), target.triple());
            assert_eq!(Target::of_isa(&*isa), Some(target));
            let writer = ObjectWriter::new(isa, test_program::source_file(), "demo.o").unwrap();
            assert_eq!(writer.target(), target);
            let bytes = writer.finish().unwrap();
            let file = object::File::parse(&*bytes).unwrap();
            let (format, architecture) = match target {
                Target::X86_64Linux => (object::BinaryFormat::Elf, object::Architecture::X86_64),
                Target::Aarch64Linux => (object::BinaryFormat::Elf, object::Architecture::Aarch64),
                Target::X86_64Windows => (object::BinaryFormat::Coff, object::Architecture::X86_64),
            };
            assert_eq!((file.format(), file.architecture()), (format, architecture));
        }
        // The optimisation level reaches Cranelift's flags.
        let isa = Target::X86_64Linux
            .isa(OptLevel::None, Cpu::Baseline)
            .unwrap();
        assert_eq!(isa.flags().opt_level(), OptLevel::None);
        assert!(isa.flags().is_pic());
    }

    #[test]
    fn test_the_host_target_is_named_by_architecture_and_system() {
        assert_eq!(
            Target::of_system("x86_64", "linux"),
            Some(Target::X86_64Linux)
        );
        assert_eq!(
            Target::of_system("aarch64", "linux"),
            Some(Target::Aarch64Linux)
        );
        assert_eq!(
            Target::of_system("x86_64", "windows"),
            Some(Target::X86_64Windows)
        );
        assert_eq!(Target::of_system("aarch64", "windows"), None);
        assert_eq!(Target::of_system("aarch64", "macos"), None);
        assert_eq!(Target::of_system("riscv64", "linux"), None);
        let expected = if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
            Some(Target::X86_64Linux)
        } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
            Some(Target::Aarch64Linux)
        } else if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
            Some(Target::X86_64Windows)
        } else {
            None
        };
        assert_eq!(Target::host(), expected);
    }

    #[test]
    fn test_only_the_host_target_detects_processor_features() {
        for target in Target::ALL {
            let isa = target.isa(OptLevel::Speed, Cpu::Host);
            if Target::host() == Some(target) {
                assert_eq!(isa.unwrap().triple().to_string(), target.triple());
            } else {
                let e = isa.err().unwrap();
                assert!(e.to_string().contains(target.triple()), "{e}");
            }
        }
    }

    #[test]
    fn test_the_writer_refuses_a_target_outside_the_three() {
        for triple in ["x86_64-apple-darwin", "x86_64-unknown-linux-musl"] {
            let isa = isa::lookup_by_name(triple)
                .unwrap()
                .finish(settings::Flags::new(settings::builder()))
                .unwrap();
            let e = ObjectWriter::new(isa, test_program::source_file(), "x.o")
                .err()
                .unwrap()
                .to_string();
            assert!(e.contains(triple), "{e}");
            assert!(e.contains("x86_64-pc-windows-msvc"), "{e}");
        }
    }

    #[test]
    fn test_positions_are_interned_below_the_default_source_location() {
        let mut positions = Positions::new(2);
        let a = positions.intern(pos(3, 5)).unwrap();
        let b = positions.intern(pos(3, 9)).unwrap();
        assert_eq!((a.bits(), b.bits()), (0, 1));
        assert_eq!(positions.intern(pos(3, 5)).unwrap(), a);
        assert_eq!(positions.get(b), Some(pos(3, 9)));
        assert_eq!(positions.get(SourceLoc::new(2)), None);
        let e = positions.intern(pos(4, 1)).err().unwrap();
        assert_eq!(
            e.to_string(),
            "a module has at most 2 distinct source positions"
        );
        // A writer's positions stop below the default SourceLoc.
        let writer = ObjectWriter::new(
            Target::X86_64Linux
                .isa(OptLevel::None, Cpu::Baseline)
                .unwrap(),
            test_program::source_file(),
            "demo.o",
        )
        .unwrap();
        assert_eq!(writer.positions.limit, SourceLoc::default().bits());
    }

    #[test]
    fn test_line_rows_follow_the_code_ranges_as_they_are() {
        let decl = pos(1, 1);
        // Code from offset 0 on has a position: no row for the declaration.
        let ranges = [
            range(0, 4, pos(2, 5)),
            range(4, 8, pos(2, 5)),
            range(8, 12, pos(2, 9)),
            range(12, 16, pos(3, 5)),
            range(16, 20, pos(2, 5)),
        ];
        assert_eq!(
            rows(decl, &ranges),
            vec![
                Row {
                    offset: 0,
                    position: pos(2, 5)
                },
                Row {
                    offset: 8,
                    position: pos(2, 9)
                },
                Row {
                    offset: 12,
                    position: pos(3, 5)
                },
                Row {
                    offset: 16,
                    position: pos(2, 5)
                },
            ]
        );
        // A prologue without a range belongs to the declaration.
        assert_eq!(
            rows(decl, &[range(1, 4, pos(2, 5))]),
            vec![
                Row {
                    offset: 0,
                    position: decl
                },
                Row {
                    offset: 1,
                    position: pos(2, 5)
                },
            ]
        );
        // A range at the declaration's own position adds no row after it.
        assert_eq!(
            rows(decl, &[range(2, 4, decl)]),
            vec![Row {
                offset: 0,
                position: decl
            }]
        );
        assert_eq!(
            rows(decl, &[]),
            vec![Row {
                offset: 0,
                position: decl
            }]
        );
    }

    /// The lines of a function's body in `demo.mn`, and its declaration line.
    pub(crate) fn expected_positions(name: &str) -> BTreeSet<SourcePos> {
        test_program::FUNCTIONS
            .iter()
            .find(|f| f.0 == name)
            .unwrap_or_else(|| panic!("no function {name}"))
            .1
            .iter()
            .map(|needle| position_of(needle))
            .collect()
    }

    /// Whether a function's positions hold every position of its body, and nothing else but
    /// perhaps its declaration, which only code without a position produces (a leaf function
    /// on AArch64 has none: the spike's finding 4).
    pub(crate) fn fits(name: &str, positions: &BTreeSet<SourcePos>) -> bool {
        let all = expected_positions(name);
        let decl = test_program::decl_of(name);
        all.iter()
            .filter(|p| **p != decl)
            .all(|p| positions.contains(p))
            && positions.is_subset(&all)
    }

    /// The same for lines alone, as CodeView holds them.
    pub(crate) fn fits_lines(name: &str, lines: &BTreeSet<u32>) -> bool {
        let all: BTreeSet<u32> = expected_positions(name).iter().map(|p| p.line).collect();
        let decl = test_program::decl_of(name).line;
        all.iter()
            .filter(|l| **l != decl)
            .all(|l| lines.contains(l))
            && lines.is_subset(&all)
    }

    #[test]
    fn test_cranelift_gives_every_function_its_positions_and_unwind_information() {
        for target in Target::ALL {
            let writer = test_program::writer(target);
            assert_eq!(writer.functions.len(), 3);
            for (f, (name, _)) in writer.functions.iter().zip(test_program::FUNCTIONS) {
                let positions: BTreeSet<SourcePos> =
                    rows(f.decl, &f.ranges).iter().map(|r| r.position).collect();
                assert!(fits(name, &positions), "{target:?} {name}: {positions:?}");
                assert_eq!(f.decl, test_program::decl_of(name));
                assert!(
                    f.ranges.iter().all(|r| r.start < r.end && r.end <= f.size),
                    "{target:?} {name}: {:?}",
                    f.ranges
                );
                assert!(
                    f.ranges.windows(2).all(|w| w[0].end <= w[1].start),
                    "{target:?} {name}: {:?}",
                    f.ranges
                );
                let kind_fits = matches!(
                    (target, &f.unwind),
                    (
                        Target::X86_64Linux | Target::Aarch64Linux,
                        Some(UnwindInfo::SystemV(_))
                    ) | (Target::X86_64Windows, Some(UnwindInfo::WindowsX64(_)))
                );
                assert!(kind_fits, "{target:?} {name}: {:?}", f.unwind);
            }
            // The object's symbols are the functions', with their code.
            let sizes: Vec<u32> = writer.functions.iter().map(|f| f.size).collect();
            let bytes = writer.finish().unwrap();
            let file = object::File::parse(&*bytes).unwrap();
            let text = file.section_by_name(".text").unwrap();
            for ((name, _), size) in test_program::FUNCTIONS.iter().zip(sizes) {
                let symbol = file.symbol_by_name(name).unwrap();
                assert!(symbol.is_definition() && symbol.is_global(), "{name}");
                assert_eq!(symbol.section_index(), Some(text.index()), "{name}");
                assert!(symbol.address() + u64::from(size) <= text.size(), "{name}");
            }
        }
    }

    /// A function `fn one() -> i64 { return 1 }` whose instruction has the location `loc`.
    fn define_one(
        writer: &mut ObjectWriter,
        loc: SourceLoc,
        decl: SourcePos,
    ) -> Result<(), CodegenError> {
        let sig = sig_i64(writer);
        let id = writer
            .module_mut()
            .declare_function("one", Linkage::Export, &sig)
            .unwrap();
        let mut ctx = return_constant(writer, &sig, id, loc, 1);
        writer.define_function(id, &mut ctx, decl)
    }

    /// A function with signature `sig` that returns `value`, from an instruction at `loc`.
    fn return_constant(
        writer: &ObjectWriter,
        sig: &cranelift_codegen::ir::Signature,
        id: FuncId,
        loc: SourceLoc,
        value: i64,
    ) -> Context {
        let frontend = writer.module().isa().frontend_config();
        let mut ctx = Context::new();
        ctx.func.signature = sig.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let mut fctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.switch_to_block(entry);
        b.seal_block(entry);
        b.set_srcloc(loc);
        let v = b.ins().iconst(types::I64, value);
        b.ins().return_(&[v]);
        b.finalize(frontend);
        ctx
    }

    #[test]
    fn test_a_source_location_the_writer_did_not_hand_out_is_refused() {
        for target in Target::ALL {
            let mut writer = test_program::empty_writer(target);
            let e = define_one(&mut writer, SourceLoc::new(7), pos(1, 1))
                .err()
                .unwrap();
            assert_eq!(
                e.to_string(),
                "one: source location @0007 was not handed out by ObjectWriter::source_loc"
            );
        }
    }

    #[test]
    fn test_a_function_defined_around_the_writer_is_refused() {
        let mut writer = test_program::empty_writer(Target::X86_64Linux);
        let sig = sig_i64(&writer);
        let id = writer
            .module_mut()
            .declare_function("around", Linkage::Export, &sig)
            .unwrap();
        // An imported function, declared but not defined, needs no debug information.
        writer
            .module_mut()
            .declare_function("imported", Linkage::Import, &sig)
            .unwrap();
        let mut ctx = return_constant(&writer, &sig, id, SourceLoc::default(), 0);
        writer.module_mut().define_function(id, &mut ctx).unwrap();
        assert_eq!(
            writer.finish().err().unwrap().to_string(),
            "around was defined without ObjectWriter::define_function, so it has no line table \
             or unwind information"
        );
        // A module whose functions all went through the writer, imports aside, is written.
        let mut writer = test_program::empty_writer(Target::X86_64Linux);
        let sig = sig_i64(&writer);
        writer
            .module_mut()
            .declare_function("imported", Linkage::Import, &sig)
            .unwrap();
        let loc = writer.source_loc(pos(2, 3)).unwrap();
        define_one(&mut writer, loc, pos(1, 1)).unwrap();
        assert!(writer.finish().is_ok());
    }

    fn sig_i64(writer: &ObjectWriter) -> cranelift_codegen::ir::Signature {
        let mut sig = writer.module().make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        sig
    }

    #[test]
    fn test_the_demo_source_names_every_position_once() {
        // Each needle the test program looks for is on exactly one line of the source.
        for (_, needles) in test_program::FUNCTIONS {
            for needle in *needles {
                assert_eq!(
                    SOURCE.lines().filter(|l| l.contains(needle)).count(),
                    1,
                    "{needle}"
                );
            }
        }
        assert_eq!(position_of("fn square"), pos(4, 1));
        assert_eq!(position_of("x * x"), pos(5, 12));
    }
}

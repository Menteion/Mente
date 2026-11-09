//! Debug and unwind information on ELF: the DWARF compilation unit, its subprograms and its
//! line table, written with `gimli`, and `.eh_frame` from Cranelift's System V unwind
//! information. Both targets, x86-64 and AArch64, are little-endian.

use std::collections::HashMap;

use cranelift_codegen::isa::unwind::UnwindInfo;
use gimli::write::{
    Address, AttributeValue, CommonInformationEntry, DwarfUnit, EhFrame, EndianVec, Expression,
    FrameTable, LineProgram, LineString, Range, RangeList, RelocateWriter, Relocation,
    RelocationTarget, Sections,
};
use gimli::{Encoding, Format, LineEncoding, RunTimeEndian, SectionId};
use object::write::{Object, Relocation as ObjectRelocation, SectionId as ObjectSection};
use object::{RelocationEncoding, RelocationFlags, RelocationKind, SectionKind};

use crate::{producer, rows, CodegenError, Function, SourceFile};

/// How `.eh_frame` encodes each FDE's address: a signed 32-bit offset from the pointer itself,
/// so that the section needs no dynamic relocation in a position-independent executable. The
/// byte is `DW_EH_PE_pcrel` (0x10) applied to `DW_EH_PE_sdata4` (0x0b).
const FDE_POINTER: gimli::DwEhPe = gimli::DwEhPe(0x1b);

/// A DWARF section as `gimli` writes it, with the relocations it asks for.
#[derive(Clone)]
struct RelocSection {
    writer: EndianVec<RunTimeEndian>,
    relocations: Vec<Relocation>,
}

impl RelocSection {
    fn new() -> Self {
        RelocSection {
            writer: EndianVec::new(RunTimeEndian::Little),
            relocations: Vec::new(),
        }
    }
}

impl RelocateWriter for RelocSection {
    type Writer = EndianVec<RunTimeEndian>;
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

/// The address of function `k`'s first byte, which a relocation against its symbol fills in.
fn start_of(k: usize) -> Address {
    Address::Symbol {
        symbol: k,
        addend: 0,
    }
}

fn object_error(e: object::write::Error) -> CodegenError {
    CodegenError::new(e.to_string())
}

/// `.eh_frame`: Cranelift's common information entry, with PC-relative FDE pointers, and an FDE
/// per function. `cranelift-object` would write absolute 64-bit pointers instead, which
/// `rust-lld` refuses in a position-independent executable (the spike's finding 2).
pub(crate) fn add_eh_frame(
    obj: &mut Object<'static>,
    fns: &[Function],
    cie: Option<CommonInformationEntry>,
) -> Result<(), CodegenError> {
    let mut cie = cie.ok_or_else(|| CodegenError::new("the ISA gives no System V CIE"))?;
    cie.fde_address_encoding = FDE_POINTER;
    let mut table = FrameTable::default();
    let cie_id = table.add_cie(cie);
    for (k, f) in fns.iter().enumerate() {
        let Some(UnwindInfo::SystemV(info)) = &f.unwind else {
            return Err(CodegenError::new(format!(
                "{}: Cranelift gave no System V unwind information",
                f.name
            )));
        };
        table.add_fde(cie_id, info.to_fde(start_of(k)));
    }
    let mut eh = EhFrame(RelocSection::new());
    table
        .write_eh_frame(&mut eh)
        .map_err(|e| CodegenError::new(e.to_string()))?;
    let section = obj.section_id(object::write::StandardSection::EhFrame);
    obj.append_section_data(section, eh.0.writer.slice(), 8);
    for r in &eh.0.relocations {
        let RelocationTarget::Symbol(k) = r.target else {
            return Err(CodegenError::new(".eh_frame refers to a section"));
        };
        // The only pointers the table holds are the FDEs' addresses, all PC-relative.
        if r.eh_pe != Some(FDE_POINTER) {
            return Err(CodegenError::new(format!(
                ".eh_frame holds a pointer encoded as {:?}",
                r.eh_pe
            )));
        }
        obj.add_relocation(
            section,
            ObjectRelocation {
                offset: r.offset as u64,
                symbol: fns[k].symbol,
                addend: r.addend,
                flags: RelocationFlags::Generic {
                    kind: RelocationKind::Relative,
                    encoding: RelocationEncoding::Generic,
                    size: r.size * 8,
                },
            },
        )
        .map_err(object_error)?;
    }
    Ok(())
}

/// The DWARF 4 compilation unit of the source file: its line program, with a sequence per
/// function, and a subprogram per function, with relocations against the function symbols and
/// between the debug sections.
pub(crate) fn add_dwarf(
    obj: &mut Object<'static>,
    fns: &[Function],
    source: &SourceFile,
) -> Result<(), CodegenError> {
    let encoding = Encoding {
        format: Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let path = || source.path.as_bytes().to_vec();
    let directory = || source.directory.as_bytes().to_vec();
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        LineEncoding::default(),
        LineString::String(directory()),
        None,
        LineString::String(path()),
        None,
    );
    let dir = program.default_directory();
    let file = program.add_file(LineString::String(path()), dir, None);
    for (k, f) in fns.iter().enumerate() {
        program.begin_sequence(Some(start_of(k)));
        let mut line = None;
        for r in rows(f.decl, &f.ranges) {
            let row = program.row();
            row.address_offset = u64::from(r.offset);
            row.file = file;
            row.line = u64::from(r.position.line);
            row.column = u64::from(r.position.column);
            // A statement starts where the line changes; a new column on the same line is not
            // one, so that debuggers step and break by line.
            row.is_statement = line != Some(r.position.line);
            line = Some(r.position.line);
            program.generate_row();
        }
        program.end_sequence(u64::from(f.size));
    }
    dwarf.unit.line_program = program;

    let ranges = RangeList(
        fns.iter()
            .enumerate()
            .map(|(k, f)| Range::StartLength {
                begin: start_of(k),
                length: u64::from(f.size),
            })
            .collect(),
    );
    let range_id = dwarf.unit.ranges.add(ranges);
    let root = dwarf.unit.root();
    let cu = dwarf.unit.get_mut(root);
    cu.set(
        gimli::DW_AT_producer,
        AttributeValue::String(producer().into_bytes()),
    );
    // Mente has no DWARF language code yet; C keeps debuggers' expression evaluators usable.
    cu.set(
        gimli::DW_AT_language,
        AttributeValue::Language(gimli::DW_LANG_C11),
    );
    cu.set(gimli::DW_AT_name, AttributeValue::String(path()));
    cu.set(gimli::DW_AT_comp_dir, AttributeValue::String(directory()));
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
            AttributeValue::String(f.debug_name.as_bytes().to_vec()),
        );
        if f.debug_name != f.name {
            e.set(
                gimli::DW_AT_linkage_name,
                AttributeValue::String(f.name.as_bytes().to_vec()),
            );
        }
        e.set(gimli::DW_AT_external, AttributeValue::Flag(true));
        e.set(
            gimli::DW_AT_decl_file,
            AttributeValue::FileIndex(Some(file)),
        );
        e.set(
            gimli::DW_AT_decl_line,
            AttributeValue::Udata(u64::from(f.decl.line)),
        );
        e.set(
            gimli::DW_AT_decl_column,
            AttributeValue::Udata(u64::from(f.decl.column)),
        );
        e.set(gimli::DW_AT_low_pc, AttributeValue::Address(start_of(k)));
        e.set(
            gimli::DW_AT_high_pc,
            AttributeValue::Udata(u64::from(f.size)),
        );
        e.set(
            gimli::DW_AT_frame_base,
            AttributeValue::Exprloc(Expression::raw(vec![gimli::DW_OP_call_frame_cfa.0])),
        );
    }

    let mut sections = Sections::new(RelocSection::new());
    dwarf
        .write(&mut sections)
        .map_err(|e| CodegenError::new(e.to_string()))?;
    let mut ids: HashMap<SectionId, ObjectSection> = HashMap::new();
    sections.for_each(|id, s| -> Result<(), CodegenError> {
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
    sections.for_each(|id, s| -> Result<(), CodegenError> {
        let Some(&section) = ids.get(&id) else {
            return Ok(());
        };
        for r in &s.relocations {
            let symbol = match r.target {
                RelocationTarget::Symbol(k) => fns[k].symbol,
                RelocationTarget::Section(target) => {
                    let t = *ids.get(&target).ok_or_else(|| {
                        CodegenError::new(format!(
                            "{} refers to an empty {}",
                            id.name(),
                            target.name()
                        ))
                    })?;
                    obj.section_symbol(t)
                }
            };
            obj.add_relocation(
                section,
                ObjectRelocation {
                    offset: r.offset as u64,
                    symbol,
                    addend: r.addend,
                    flags: RelocationFlags::Generic {
                        kind: RelocationKind::Absolute,
                        encoding: RelocationEncoding::Generic,
                        size: r.size * 8,
                    },
                },
            )
            .map_err(object_error)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use crate::test_program::{self, decl_of, FUNCTIONS};
    use crate::tests::fits;
    use crate::{SourcePos, Target};
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
    use std::collections::BTreeSet;

    type Reader<'a> = gimli::EndianSlice<'a, gimli::LittleEndian>;

    /// Each relocation of a section: its offset, kind, size in bits, addend and the name of
    /// the symbol it targets, or of the section a section symbol stands for.
    fn relocations(
        file: &object::File<'_>,
        name: &str,
    ) -> Vec<(u64, object::RelocationKind, u8, i64, String)> {
        let section = file.section_by_name(name).unwrap();
        section
            .relocations()
            .map(|(offset, r)| {
                let object::RelocationTarget::Symbol(index) = r.target() else {
                    panic!("{name}: a relocation without a symbol");
                };
                let symbol = file.symbol_by_index(index).unwrap();
                let target = match symbol.kind() {
                    object::SymbolKind::Section => file
                        .section_by_index(symbol.section_index().unwrap())
                        .unwrap()
                        .name()
                        .unwrap()
                        .to_string(),
                    _ => symbol.name().unwrap().to_string(),
                };
                (offset, r.kind(), r.size(), r.addend(), target)
            })
            .collect()
    }

    fn load<'a>(file: &'a object::File<'a>) -> gimli::Dwarf<Reader<'a>> {
        gimli::Dwarf::load(|id: gimli::SectionId| -> Result<Reader<'a>, gimli::Error> {
            Ok(gimli::EndianSlice::new(
                file.section_by_name(id.name())
                    .and_then(|s| s.data().ok())
                    .unwrap_or(&[]),
                gimli::LittleEndian,
            ))
        })
        .unwrap()
    }

    /// One row of a line program read back: its offset in the sequence, position and whether
    /// it starts a statement.
    #[derive(Debug, PartialEq, Eq)]
    struct ReadRow {
        offset: u64,
        position: SourcePos,
        is_stmt: bool,
    }

    /// A function defined under a name of its own: DWARF names it so, and keeps its symbol as
    /// the linkage name.
    #[test]
    fn test_dwarf_names_a_function_as_it_was_defined() {
        for target in [Target::X86_64Linux, Target::Aarch64Linux] {
            let bytes = test_program::writer_with_one_named(target, "one")
                .finish()
                .unwrap();
            let file = object::File::parse(&*bytes).unwrap();
            let dwarf = load(&file);
            let header = dwarf.units().next().unwrap().unwrap();
            let unit = dwarf.unit(header).unwrap();
            let mut entries = unit.entries();
            entries.next_dfs().unwrap();
            let sp = entries.next_dfs().unwrap().unwrap();
            let string = |at| {
                dwarf
                    .attr_string(&unit, sp.attr_value(at).unwrap())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };
            assert_eq!(string(gimli::DW_AT_name), "one", "{target:?}");
            assert_eq!(
                string(gimli::DW_AT_linkage_name),
                test_program::NAMED_SYMBOL,
                "{target:?}"
            );
        }
    }

    #[test]
    fn test_elf_object_carries_dwarf_lines_and_subprograms() {
        for target in [Target::X86_64Linux, Target::Aarch64Linux] {
            let (bytes, fns) = test_program::object(target);
            let file = object::File::parse(&*bytes).unwrap();
            let dwarf = load(&file);
            let header = dwarf.units().next().unwrap().unwrap();
            let unit = dwarf.unit(header).unwrap();
            let string = |value: gimli::AttributeValue<Reader<'_>>| {
                dwarf
                    .attr_string(&unit, value)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };

            // The line program: a sequence per function, in order, ending at its size.
            let program = unit.line_program.clone().unwrap();
            let mut rows = program.rows();
            let mut sequences: Vec<Vec<ReadRow>> = vec![Vec::new()];
            let mut ends = Vec::new();
            while let Some((_, row)) = rows.next_row().unwrap() {
                if row.end_sequence() {
                    ends.push(row.address());
                    sequences.push(Vec::new());
                    continue;
                }
                let column = match row.column() {
                    gimli::ColumnType::LeftEdge => 0,
                    gimli::ColumnType::Column(c) => c.get() as u32,
                };
                sequences.last_mut().unwrap().push(ReadRow {
                    offset: row.address(),
                    position: SourcePos {
                        line: row.line().unwrap().get() as u32,
                        column,
                    },
                    is_stmt: row.is_stmt(),
                });
            }
            sequences.pop();
            assert_eq!(sequences.len(), fns.len(), "{target:?}");
            for ((f, (name, _)), (seq, end)) in
                fns.iter().zip(FUNCTIONS).zip(sequences.iter().zip(&ends))
            {
                // In an unlinked object every sequence starts at address 0, which its relocation
                // moves to the function.
                assert_eq!(*end, u64::from(f.size), "{target:?} {name}");
                let expected: Vec<ReadRow> = {
                    let mut line = None;
                    crate::rows(f.decl, &f.ranges)
                        .into_iter()
                        .map(|r| {
                            let is_stmt = line != Some(r.position.line);
                            line = Some(r.position.line);
                            ReadRow {
                                offset: u64::from(r.offset),
                                position: r.position,
                                is_stmt,
                            }
                        })
                        .collect()
                };
                assert_eq!(seq, &expected, "{target:?} {name}");
                let positions: BTreeSet<SourcePos> = seq.iter().map(|r| r.position).collect();
                assert!(fits(name, &positions), "{target:?} {name}: {positions:?}");
                // square's line 5 holds two columns: the multiplication and the return.
                if *name == "square" {
                    let line5: Vec<&ReadRow> =
                        seq.iter().filter(|r| r.position.line == 5).collect();
                    assert!(line5.len() >= 2, "{target:?}: {seq:?}");
                    assert!(
                        line5[0].is_stmt && line5[1..].iter().all(|r| !r.is_stmt),
                        "{target:?}: {seq:?}"
                    );
                }
            }

            // The compilation unit and a subprogram per function.
            let mut entries = unit.entries();
            let cu = entries.next_dfs().unwrap().unwrap();
            assert_eq!(cu.tag(), gimli::DW_TAG_compile_unit);
            assert_eq!(string(cu.attr_value(gimli::DW_AT_name).unwrap()), "demo.mn");
            assert_eq!(
                string(cu.attr_value(gimli::DW_AT_comp_dir).unwrap()),
                "/src"
            );
            assert_eq!(
                string(cu.attr_value(gimli::DW_AT_producer).unwrap()),
                format!(
                    "mente-codegen {} (Cranelift 0.137.1)",
                    env!("CARGO_PKG_VERSION")
                )
            );
            let mut subprograms = Vec::new();
            while let Some(e) = entries.next_dfs().unwrap() {
                assert_eq!(e.tag(), gimli::DW_TAG_subprogram);
                // Named by its symbol, a function needs no linkage name of its own.
                assert!(e.attr_value(gimli::DW_AT_linkage_name).is_none());
                let udata = |at| match e.attr_value(at) {
                    Some(gimli::AttributeValue::Udata(v)) => v,
                    other => panic!("{at}: {other:?}"),
                };
                subprograms.push((
                    string(e.attr_value(gimli::DW_AT_name).unwrap()),
                    SourcePos {
                        line: udata(gimli::DW_AT_decl_line) as u32,
                        column: udata(gimli::DW_AT_decl_column) as u32,
                    },
                    udata(gimli::DW_AT_high_pc),
                ));
            }
            let expected: Vec<(String, SourcePos, u64)> = fns
                .iter()
                .zip(FUNCTIONS)
                .map(|(f, (name, _))| (name.to_string(), decl_of(name), u64::from(f.size)))
                .collect();
            assert_eq!(subprograms, expected, "{target:?}");

            // Each address is relocated against its function: a subprogram's start in
            // .debug_info, a sequence's in .debug_line, and a range's start and end in
            // .debug_ranges. The unit refers to the other sections through their section
            // symbols.
            let to_functions = |section: &str| -> Vec<(String, i64)> {
                relocations(&file, section)
                    .into_iter()
                    .filter(|r| {
                        assert_eq!(r.1, object::RelocationKind::Absolute, "{section}: {r:?}");
                        !r.4.starts_with(".debug")
                    })
                    .map(|r| {
                        assert_eq!(r.2, 64, "{section}: {r:?}");
                        (r.4, r.3)
                    })
                    .collect()
            };
            let starts: Vec<(String, i64)> =
                FUNCTIONS.iter().map(|(n, _)| (n.to_string(), 0)).collect();
            assert_eq!(to_functions(".debug_info"), starts, "{target:?}");
            assert_eq!(to_functions(".debug_line"), starts, "{target:?}");
            let ranges: Vec<(String, i64)> = fns
                .iter()
                .zip(FUNCTIONS)
                .flat_map(|(f, (n, _))| [(n.to_string(), 0), (n.to_string(), i64::from(f.size))])
                .collect();
            assert_eq!(to_functions(".debug_ranges"), ranges, "{target:?}");
            let to_sections: BTreeSet<(String, u8)> = relocations(&file, ".debug_info")
                .into_iter()
                .filter(|r| r.4.starts_with(".debug"))
                .map(|r| (r.4, r.2))
                .collect();
            assert_eq!(
                to_sections,
                [
                    ".debug_abbrev",
                    ".debug_line",
                    ".debug_ranges",
                    ".debug_str"
                ]
                .into_iter()
                .filter(|s| file.section_by_name(s).is_some())
                .map(|s| (s.to_string(), 32))
                .collect(),
                "{target:?}"
            );
        }
    }

    #[test]
    fn test_elf_eh_frame_has_pc_relative_fde_pointers() {
        for target in [Target::X86_64Linux, Target::Aarch64Linux] {
            let (bytes, fns) = test_program::object(target);
            let file = object::File::parse(&*bytes).unwrap();
            // One 32-bit PC-relative relocation per FDE, against its function, in order.
            let found: Vec<(object::RelocationKind, u8, String)> = relocations(&file, ".eh_frame")
                .into_iter()
                .map(|r| (r.1, r.2, r.4))
                .collect();
            let expected: Vec<(object::RelocationKind, u8, String)> = FUNCTIONS
                .iter()
                .map(|(n, _)| (object::RelocationKind::Relative, 32, n.to_string()))
                .collect();
            assert_eq!(found, expected, "{target:?}");

            // Read the frame table back: one CIE whose FDEs encode their pointers PC-relative,
            // and an FDE per function covering its code.
            use gimli::UnwindSection as _;
            let data = file.section_by_name(".eh_frame").unwrap().data().unwrap();
            let eh = gimli::EhFrame::new(data, gimli::LittleEndian);
            let bases = gimli::BaseAddresses::default().set_eh_frame(0);
            let mut entries = eh.entries(&bases);
            let mut lengths = Vec::new();
            let mut cies = 0;
            while let Some(entry) = entries.next().unwrap() {
                match entry {
                    gimli::CieOrFde::Cie(cie) => {
                        cies += 1;
                        assert_eq!(cie.fde_address_encoding(), Some(super::FDE_POINTER));
                    }
                    gimli::CieOrFde::Fde(partial) => {
                        let fde = partial
                            .parse(|_, bases, offset| eh.cie_from_offset(bases, offset))
                            .unwrap();
                        lengths.push(fde.len());
                    }
                }
            }
            assert_eq!(cies, 1, "{target:?}");
            let sizes: Vec<u64> = fns.iter().map(|f| u64::from(f.size)).collect();
            assert_eq!(lengths, sizes, "{target:?}");
            // The encoding is a 32-bit signed value, relative to the pointer's own address.
            assert_eq!(super::FDE_POINTER.application(), gimli::DW_EH_PE_pcrel);
            assert_eq!(super::FDE_POINTER.format(), gimli::DW_EH_PE_sdata4);
        }
    }
}

//! Debug and unwind information on COFF for x86-64 Windows: `.pdata` and `.xdata` from
//! Cranelift's Windows x64 unwind information, and the CodeView symbols and line tables of
//! `.debug$S`, which the linker turns into a PDB. No Rust crate writes CodeView, so its records
//! are encoded here.
//!
//! CodeView line tables hold lines without columns, as `clang-cl` writes them by default; the
//! columns are in DWARF only.

use cranelift_codegen::isa::unwind::UnwindInfo;
use object::pe::{
    RelocationType, IMAGE_REL_AMD64_ADDR32NB, IMAGE_REL_AMD64_SECREL, IMAGE_REL_AMD64_SECTION,
};
use object::write::{Object, Relocation as ObjectRelocation, SymbolId};
use object::{RelocationFlags, SectionKind};

use crate::{producer, rows, CodeRange, CodegenError, Function, SourceFile, SourcePos};

/// The largest line a CodeView line entry holds: its line field is 24 bits wide.
const MAX_LINE: u32 = 0x00FF_FFFF;
/// Bit 31 of a line entry: the line starts a statement.
const LINE_IS_STATEMENT: u32 = 0x8000_0000;

const CV_SIGNATURE_C13: u32 = 4;
const CV_CFL_AMD64: u16 = 0xD0;
const S_END: u16 = 0x0006;
const S_OBJNAME: u16 = 0x1101;
const S_GPROC32: u16 = 0x1110;
const S_COMPILE3: u16 = 0x113C;
const DEBUG_S_SYMBOLS: u32 = 0xF1;
const DEBUG_S_LINES: u32 = 0xF2;
const DEBUG_S_STRINGTABLE: u32 = 0xF3;
const DEBUG_S_FILECHKSMS: u32 = 0xF4;

fn coff(typ: RelocationType) -> RelocationFlags {
    RelocationFlags::Coff { typ }
}

fn object_error(e: object::write::Error) -> CodegenError {
    CodegenError::new(e.to_string())
}

/// `.xdata` holds each function's UNWIND_INFO, as Cranelift encodes it, at an offset aligned to
/// four bytes; `.pdata` holds a RUNTIME_FUNCTION per function, whose three image-relative
/// addresses (the function's start and end, and its UNWIND_INFO) the linker fills in.
pub(crate) fn add_unwind_info(
    obj: &mut Object<'static>,
    fns: &[Function],
) -> Result<(), CodegenError> {
    let xdata = obj.add_section(Vec::new(), b".xdata".to_vec(), SectionKind::ReadOnlyData);
    let pdata = obj.add_section(Vec::new(), b".pdata".to_vec(), SectionKind::ReadOnlyData);
    let xdata_symbol = obj.section_symbol(xdata);
    let mut x = Vec::new();
    let mut p = Vec::new();
    let mut relocations: Vec<(u64, SymbolId, i64)> = Vec::new();
    for f in fns {
        let Some(UnwindInfo::WindowsX64(info)) = &f.unwind else {
            return Err(CodegenError::new(format!(
                "{}: Cranelift gave no Windows x64 unwind information",
                f.name
            )));
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
        relocations.push((entry, f.symbol, 0));
        relocations.push((entry + 4, f.symbol, i64::from(f.size)));
        relocations.push((entry + 8, xdata_symbol, at as i64));
    }
    obj.set_section_data(xdata, x, 4);
    obj.set_section_data(pdata, p, 4);
    for (offset, symbol, addend) in relocations {
        obj.add_relocation(
            pdata,
            ObjectRelocation {
                offset,
                symbol,
                addend,
                flags: coff(IMAGE_REL_AMD64_ADDR32NB),
            },
        )
        .map_err(object_error)?;
    }
    Ok(())
}

/// One CodeView symbol record: its length, kind and payload, padded to four bytes. `out` is
/// four-byte aligned before and after.
fn cv_record(out: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    let start = out.len();
    out.extend_from_slice(&[0, 0]); // the length, written below
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(payload);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    // The length counts the bytes after itself.
    let length = (out.len() - start - 2) as u16;
    out[start..start + 2].copy_from_slice(&length.to_le_bytes());
}

/// One subsection of `.debug$S`: its kind, length and data, padded to four bytes. Returns the
/// offset of its data.
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

/// The rows of a function's CodeView line table: its line rows, with a row only where the line
/// changes.
fn line_rows(decl: SourcePos, ranges: &[CodeRange]) -> Vec<(u32, u32)> {
    let mut lines: Vec<(u32, u32)> = Vec::new();
    for r in rows(decl, ranges) {
        if lines
            .last()
            .is_none_or(|(_, line)| *line != r.position.line)
        {
            lines.push((r.offset, r.position.line));
        }
    }
    lines
}

/// `.debug$S`: the object's name and producer, a procedure per function, the file table, and a
/// line table per function. Each procedure and line table names its function's offset and
/// section through SECREL and SECTION relocations.
pub(crate) fn add_codeview(
    obj: &mut Object<'static>,
    fns: &[Function],
    source: &SourceFile,
    object_name: &str,
) -> Result<(), CodegenError> {
    let mut relocations: Vec<(u64, SymbolId, RelocationType)> = Vec::new();
    let mut out = CV_SIGNATURE_C13.to_le_bytes().to_vec();

    let mut syms = Vec::new();
    let mut objname = 0u32.to_le_bytes().to_vec(); // no signature
    objname.extend_from_slice(object_name.as_bytes());
    objname.push(0);
    cv_record(&mut syms, S_OBJNAME, &objname);
    let mut compile = 0u32.to_le_bytes().to_vec(); // language 0 (C), no flags
    compile.extend_from_slice(&CV_CFL_AMD64.to_le_bytes());
    // The front end's version (mente-codegen 0.1.0.0), then the back end's (Cranelift 0.137.1.0).
    for v in [0u16, 1, 0, 0, 0, 137, 1, 0] {
        compile.extend_from_slice(&v.to_le_bytes());
    }
    compile.extend_from_slice(producer().as_bytes());
    compile.push(0);
    cv_record(&mut syms, S_COMPILE3, &compile);
    let mut fixups = Vec::new();
    for f in fns {
        let mut p = Vec::new();
        // parent, end, next, length, debug start, debug end, type
        for v in [0u32, 0, 0, f.size, 0, f.size, 0] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let offset_at = p.len();
        p.extend_from_slice(&0u32.to_le_bytes()); // the offset, relocated
        p.extend_from_slice(&0u16.to_le_bytes()); // the section, relocated
        p.push(0); // flags
        p.extend_from_slice(f.name.as_bytes());
        p.push(0);
        let record_at = syms.len() + 4; // past the record's length and kind
        cv_record(&mut syms, S_GPROC32, &p);
        fixups.push((record_at + offset_at, f.symbol));
        cv_record(&mut syms, S_END, &[]);
    }
    let syms_at = cv_subsection(&mut out, DEBUG_S_SYMBOLS, &syms);
    for (at, symbol) in fixups {
        relocations.push(((syms_at + at) as u64, symbol, IMAGE_REL_AMD64_SECREL));
        relocations.push(((syms_at + at + 4) as u64, symbol, IMAGE_REL_AMD64_SECTION));
    }

    let mut strings = vec![0u8];
    strings.extend_from_slice(source.path.as_bytes());
    strings.push(0);
    cv_subsection(&mut out, DEBUG_S_STRINGTABLE, &strings);
    let mut checksums = 1u32.to_le_bytes().to_vec(); // the path's offset in the string table
    checksums.extend_from_slice(&[0, 0, 0, 0]); // no checksum, and padding
    cv_subsection(&mut out, DEBUG_S_FILECHKSMS, &checksums);

    for f in fns {
        let rows = line_rows(f.decl, &f.ranges);
        let mut lines = Vec::new();
        lines.extend_from_slice(&0u32.to_le_bytes()); // the offset, relocated
        lines.extend_from_slice(&0u16.to_le_bytes()); // the section, relocated
        lines.extend_from_slice(&0u16.to_le_bytes()); // flags: no columns
        lines.extend_from_slice(&f.size.to_le_bytes());
        lines.extend_from_slice(&0u32.to_le_bytes()); // the file: offset 0 in the checksums
        lines.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        lines.extend_from_slice(&(12 + 8 * rows.len() as u32).to_le_bytes());
        for (offset, line) in rows {
            if line > MAX_LINE {
                return Err(CodegenError::new(format!(
                    "{}: CodeView holds lines up to {MAX_LINE}, not {line}",
                    f.name
                )));
            }
            lines.extend_from_slice(&offset.to_le_bytes());
            // The line is below bit 24, so adding the flag sets bit 31.
            lines.extend_from_slice(&(line + LINE_IS_STATEMENT).to_le_bytes());
        }
        let at = cv_subsection(&mut out, DEBUG_S_LINES, &lines) as u64;
        relocations.push((at, f.symbol, IMAGE_REL_AMD64_SECREL));
        relocations.push((at + 4, f.symbol, IMAGE_REL_AMD64_SECTION));
    }

    let section = obj.add_section(Vec::new(), b".debug$S".to_vec(), SectionKind::Debug);
    obj.set_section_data(section, out, 4);
    for (offset, symbol, typ) in relocations {
        obj.add_relocation(
            section,
            ObjectRelocation {
                offset,
                symbol,
                addend: 0,
                flags: coff(typ),
            },
        )
        .map_err(object_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_program::{self, FUNCTIONS};
    use crate::tests::fits_lines;
    use crate::Target;
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
    use std::collections::BTreeSet;

    fn u16_at(data: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(data[at..at + 2].try_into().unwrap())
    }

    fn u32_at(data: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
    }

    fn c_string(data: &[u8], at: usize) -> String {
        let end = at + data[at..].iter().position(|b| *b == 0).unwrap();
        String::from_utf8(data[at..end].to_vec()).unwrap()
    }

    /// Each relocation of a section: its offset, COFF type, and the name of the symbol it
    /// targets, or of the section a section symbol stands for.
    fn relocations(file: &object::File<'_>, name: &str) -> Vec<(u64, u16, String)> {
        file.section_by_name(name)
            .unwrap()
            .relocations()
            .map(|(offset, r)| {
                let object::RelocationFlags::Coff { typ } = r.flags() else {
                    panic!("{name}: not a COFF relocation");
                };
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
                (offset, typ.0, target)
            })
            .collect()
    }

    #[test]
    fn test_coff_object_carries_pdata_and_xdata() {
        let (bytes, fns) = test_program::object(Target::X86_64Windows);
        let file = object::File::parse(&*bytes).unwrap();
        let pdata = file.section_by_name(".pdata").unwrap().data().unwrap();
        let xdata = file.section_by_name(".xdata").unwrap().data().unwrap();
        assert_eq!(pdata.len(), 12 * fns.len());
        // Three image-relative addresses per RUNTIME_FUNCTION: the function's start and end,
        // and its UNWIND_INFO in .xdata. COFF keeps each addend in the section's data.
        let found = relocations(&file, ".pdata");
        let mut expected = Vec::new();
        for (k, (name, _)) in FUNCTIONS.iter().enumerate() {
            let entry = 12 * k as u64;
            for (offset, target) in [(0, *name), (4, name), (8, ".xdata")] {
                expected.push((
                    entry + offset,
                    IMAGE_REL_AMD64_ADDR32NB.0,
                    target.to_string(),
                ));
            }
        }
        assert_eq!(found, expected);
        let mut next = 0;
        for (k, f) in fns.iter().enumerate() {
            assert_eq!(u32_at(pdata, 12 * k), 0);
            assert_eq!(u32_at(pdata, 12 * k + 4), f.size);
            let at = u32_at(pdata, 12 * k + 8) as usize;
            // Each UNWIND_INFO starts on four bytes, after the one before it: version 1, a
            // prologue inside the function, and its unwind codes, padded to an even count.
            assert!(at.is_multiple_of(4) && at >= next, "{k}: {at}");
            assert_eq!(xdata[at] & 0x07, 1, "{k}");
            assert!(u32::from(xdata[at + 1]) <= f.size, "{k}");
            let codes = usize::from(xdata[at + 2]);
            next = at + 4 + 2 * codes.next_multiple_of(2);
            let Some(UnwindInfo::WindowsX64(info)) = &f.unwind else {
                panic!("{k}: no Windows unwind information");
            };
            let mut emitted = vec![0u8; info.emit_size()];
            info.emit(&mut emitted);
            assert_eq!(&xdata[at..at + emitted.len()], &emitted[..], "{k}");
        }
        assert_eq!(next, xdata.len());
    }

    /// A `.debug$S` subsection read back: its kind, the offset of its data, and its data.
    fn subsections(data: &[u8]) -> Vec<(u32, usize, &[u8])> {
        assert_eq!(u32_at(data, 0), CV_SIGNATURE_C13);
        let mut at = 4;
        let mut found = Vec::new();
        while at < data.len() {
            let kind = u32_at(data, at);
            let len = u32_at(data, at + 4) as usize;
            found.push((kind, at + 8, &data[at + 8..at + 8 + len]));
            at += 8 + len.next_multiple_of(4);
        }
        assert_eq!(at, data.len());
        found
    }

    #[test]
    fn test_coff_codeview_names_procedures_and_lines() {
        let (bytes, fns) = test_program::object(Target::X86_64Windows);
        let file = object::File::parse(&*bytes).unwrap();
        let data = file.section_by_name(".debug$S").unwrap().data().unwrap();
        let subs = subsections(data);
        let kinds: Vec<u32> = subs.iter().map(|s| s.0).collect();
        let mut expected_kinds = vec![DEBUG_S_SYMBOLS, DEBUG_S_STRINGTABLE, DEBUG_S_FILECHKSMS];
        expected_kinds.extend(fns.iter().map(|_| DEBUG_S_LINES));
        assert_eq!(kinds, expected_kinds);
        let mut expected_relocations = Vec::new();

        // The symbols: the object's name, the producer, and a procedure per function.
        let (_, syms_at, syms) = subs[0];
        let mut records = Vec::new();
        let mut at = 0;
        while at < syms.len() {
            let length = usize::from(u16_at(syms, at));
            assert!((length + 2).is_multiple_of(4), "record at {at}");
            records.push((u16_at(syms, at + 2), at + 4, &syms[at + 4..at + 2 + length]));
            at += 2 + length;
        }
        assert_eq!(at, syms.len());
        assert_eq!(records.len(), 2 + 2 * fns.len());
        assert_eq!(records[0].0, S_OBJNAME);
        assert_eq!(c_string(records[0].2, 4), "demo.obj");
        assert_eq!(records[1].0, S_COMPILE3);
        assert_eq!(u16_at(records[1].2, 4), CV_CFL_AMD64);
        assert_eq!(
            c_string(records[1].2, 22),
            format!(
                "mente-codegen {} (Cranelift 0.137.1)",
                env!("CARGO_PKG_VERSION")
            )
        );
        for (k, (f, (name, _))) in fns.iter().zip(FUNCTIONS).enumerate() {
            let (kind, payload_at, payload) = records[2 + 2 * k];
            assert_eq!(kind, S_GPROC32, "{name}");
            assert_eq!(u32_at(payload, 12), f.size, "{name}: length");
            assert_eq!(u32_at(payload, 20), f.size, "{name}: debug end");
            assert_eq!(c_string(payload, 35), *name);
            assert_eq!(records[3 + 2 * k].0, S_END, "{name}");
            let offset_at = (syms_at + payload_at + 28) as u64;
            expected_relocations.push((offset_at, IMAGE_REL_AMD64_SECREL.0, name.to_string()));
            expected_relocations.push((offset_at + 4, IMAGE_REL_AMD64_SECTION.0, name.to_string()));
        }

        // The one file: its path in the string table, at the offset the checksums give.
        let strings = subs[1].2;
        let checksums = subs[2].2;
        assert_eq!(checksums.len(), 8);
        assert_eq!(c_string(strings, u32_at(checksums, 0) as usize), "demo.mn");

        // A line table per function, covering its code, with a row where the line changes.
        for (k, (f, (name, _))) in fns.iter().zip(FUNCTIONS).enumerate() {
            let (_, lines_at, body) = subs[3 + k];
            assert_eq!(u16_at(body, 6), 0, "{name}: flags");
            assert_eq!(u32_at(body, 8), f.size, "{name}: code size");
            assert_eq!(u32_at(body, 12), 0, "{name}: file");
            let n = u32_at(body, 16) as usize;
            assert_eq!(u32_at(body, 20) as usize, 12 + 8 * n, "{name}: block size");
            assert_eq!(body.len(), 24 + 8 * n, "{name}");
            let rows: Vec<(u32, u32)> = (0..n)
                .map(|j| (u32_at(body, 24 + 8 * j), u32_at(body, 28 + 8 * j)))
                .collect();
            assert!(
                rows.iter().all(|(_, l)| l & LINE_IS_STATEMENT != 0),
                "{name}: {rows:?}"
            );
            let lines: Vec<(u32, u32)> = rows.iter().map(|(o, l)| (*o, l & MAX_LINE)).collect();
            assert_eq!(lines[0].0, 0, "{name}: {lines:?}");
            assert!(
                lines
                    .windows(2)
                    .all(|w| w[0].0 < w[1].0 && w[0].1 != w[1].1),
                "{name}: {lines:?}"
            );
            assert!(lines.iter().all(|(o, _)| *o < f.size), "{name}: {lines:?}");
            let set: BTreeSet<u32> = lines.iter().map(|(_, l)| *l).collect();
            assert!(fits_lines(name, &set), "{name}: {set:?}");
            let at = lines_at as u64;
            expected_relocations.push((at, IMAGE_REL_AMD64_SECREL.0, name.to_string()));
            expected_relocations.push((at + 4, IMAGE_REL_AMD64_SECTION.0, name.to_string()));
        }
        assert_eq!(relocations(&file, ".debug$S"), expected_relocations);
    }

    #[test]
    fn test_codeview_line_rows_keep_one_row_per_line() {
        let decl = SourcePos { line: 1, column: 1 };
        let at = |line, column| SourcePos { line, column };
        let ranges: Vec<CodeRange> = [
            (2, at(2, 5)),
            (6, at(2, 9)),
            (10, at(3, 5)),
            (14, at(3, 5)),
            (16, at(2, 5)),
        ]
        .into_iter()
        .map(|(start, position)| CodeRange {
            start,
            end: start + 2,
            position,
        })
        .collect();
        assert_eq!(
            line_rows(decl, &ranges),
            vec![(0, 1), (2, 2), (10, 3), (16, 2)]
        );
    }

    #[test]
    fn test_codeview_refuses_lines_it_cannot_hold() {
        for (line, holds) in [(MAX_LINE, true), (MAX_LINE + 1, false)] {
            let decl = SourcePos { line, column: 1 };
            for target in Target::ALL {
                let writer = test_program::writer_with_one(target, decl);
                let result = writer.finish();
                if target == Target::X86_64Windows && !holds {
                    assert_eq!(
                        result.err().unwrap().to_string(),
                        "one: CodeView holds lines up to 16777215, not 16777216"
                    );
                } else {
                    // DWARF holds any line.
                    assert!(result.is_ok(), "{target:?} {line}");
                }
            }
        }
        // The largest line CodeView holds is read back as it was given.
        let decl = SourcePos {
            line: MAX_LINE,
            column: 1,
        };
        let bytes = test_program::writer_with_one(Target::X86_64Windows, decl)
            .finish()
            .unwrap();
        let file = object::File::parse(&*bytes).unwrap();
        let data = file.section_by_name(".debug$S").unwrap().data().unwrap();
        let (_, _, body) = subsections(data)[3];
        assert_eq!(u32_at(body, 28), MAX_LINE + LINE_IS_STATEMENT);
    }
}

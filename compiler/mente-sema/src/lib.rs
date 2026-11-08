//! `mente-sema` — Semantic Prover: 64B Cache-Line Layout, `bitstruct`, `@total`/`decreases`, and Micro-Arch Contracts.
//!
//! Governed by `spec/morphic-layouts.md`, `spec/hardware-contracts.md` and
//! `spec/ghost-contracts-and-totality.md`. The `@total` and `@branchless` checks here are
//! token scans over unparsed bodies, not proofs; see `docs/STATUS.md`.

use std::collections::HashMap;

use mente_syntax::{BitStructDecl, FnDecl, ItemDecl, ModuleFile, ParamMode, StructDecl, TokenKind};

#[derive(Clone, Debug)]
pub struct LayoutProof {
    pub struct_name: String,
    pub total_bytes: usize,
    pub align_bytes: usize,
    pub markdown_table: String,
}

#[derive(Clone, Debug)]
pub struct BitStructProof {
    pub name: String,
    pub total_bits: usize,
}

#[derive(Clone, Debug)]
pub struct ModuleVerificationReport {
    pub module_path: String,
    pub layout_proofs: Vec<LayoutProof>,
    pub bitstruct_proofs: Vec<BitStructProof>,
    pub total_functions_verified: usize,
    pub branchless_functions_verified: usize,
    pub kernel_functions_verified: usize,
    pub examples_count: usize,
}

/// Runs full semantic & hardware layout verification on a parsed `.mn` `ModuleFile`.
pub fn verify_module(module: &ModuleFile) -> Result<ModuleVerificationReport, Vec<String>> {
    let mut errors = Vec::new();
    let mut layout_proofs = Vec::new();
    let mut bitstruct_proofs = Vec::new();
    let mut total_functions_verified = 0usize;
    let mut branchless_functions_verified = 0usize;
    let mut kernel_functions_verified = 0usize;
    let mut examples_count = 0usize;

    // Byte sizes and alignments of the module's own fixed-size types, so a `@layout` field
    // may name them, and bit widths of its bitstructs, so a bitstruct field may nest one.
    let mut named_sizes: HashMap<String, usize> = HashMap::new();
    let mut named_aligns: HashMap<String, usize> = HashMap::new();
    let mut bitstruct_bits: HashMap<String, usize> = HashMap::new();
    for item in &module.items {
        match item {
            ItemDecl::BitStruct(bs) => {
                if let Some(bits) = backing_bits(&bs.backing_type) {
                    named_sizes.insert(bs.name.clone(), bits / 8);
                    named_aligns.insert(bs.name.clone(), bits / 8);
                    bitstruct_bits.insert(bs.name.clone(), bits);
                }
            }
            ItemDecl::Struct(s) => {
                if let Some((size, align)) = extract_layout_attr(s) {
                    named_sizes.insert(s.name.clone(), size);
                    named_aligns.insert(s.name.clone(), align);
                }
            }
            _ => {}
        }
    }

    for item in &module.items {
        match item {
            ItemDecl::Struct(s) => {
                if let Some((size, align)) = extract_layout_attr(s) {
                    match verify_explicit_struct_layout(s, size, align, &named_sizes, &named_aligns)
                    {
                        Ok(proof) => layout_proofs.push(proof),
                        Err(e) => errors.push(e),
                    }
                }
            }
            ItemDecl::BitStruct(bs) => match verify_bitstruct_layout(bs, &bitstruct_bits) {
                Ok(proof) => bitstruct_proofs.push(proof),
                Err(e) => errors.push(e),
            },
            ItemDecl::Fn(f) => {
                if let Err(e) = verify_function_contracts(f) {
                    errors.push(e);
                } else {
                    if f.attrs.iter().any(|a| a.name == "total") || !f.decreases.is_empty() {
                        total_functions_verified += 1;
                    }
                    if f.attrs.iter().any(|a| a.name == "branchless") {
                        branchless_functions_verified += 1;
                    }
                    if f.attrs.iter().any(|a| a.name == "kernel") {
                        kernel_functions_verified += 1;
                    }
                }
            }
            ItemDecl::Example(_) => {
                examples_count += 1;
            }
            _ => {}
        }
    }

    if errors.is_empty() {
        Ok(ModuleVerificationReport {
            module_path: module.module_path.clone(),
            layout_proofs,
            bitstruct_proofs,
            total_functions_verified,
            branchless_functions_verified,
            kernel_functions_verified,
            examples_count,
        })
    } else {
        Err(errors)
    }
}

fn extract_layout_attr(s: &StructDecl) -> Option<(usize, usize)> {
    for a in &s.attrs {
        if a.name == "layout" {
            let mut size = None;
            let mut align = None;
            for part in a.args.split(',') {
                let kv: Vec<&str> = part.split(':').collect();
                if kv.len() == 2 {
                    let k = kv[0].trim();
                    let v = kv[1].trim().parse::<usize>().ok();
                    if k == "size" {
                        size = v;
                    } else if k == "align" {
                        align = v;
                    }
                }
            }
            if let (Some(sz), Some(al)) = (size, align) {
                return Some((sz, al));
            }
        }
    }
    None
}

/// Proves that `@layout(size: S, align: A)` has contiguous, hole-free `@bytes(start..end)` fields summing to `S`,
/// each at an offset its type's alignment allows unless the struct is `@packed`.
pub fn verify_explicit_struct_layout(
    s: &StructDecl,
    expected_size: usize,
    align: usize,
    named_sizes: &HashMap<String, usize>,
    named_aligns: &HashMap<String, usize>,
) -> Result<LayoutProof, String> {
    if !align.is_power_of_two() {
        return Err(format!(
            "E0410 [Layout Error]: struct `{}` alignment {} is not a power of two",
            s.name, align
        ));
    }
    if !expected_size.is_multiple_of(align) {
        return Err(format!(
            "E0411 [Layout Error]: struct `{}` size {} is not a multiple of alignment {}",
            s.name, expected_size, align
        ));
    }

    let packed = s.attrs.iter().any(|a| a.name == "packed");
    let mut cursor = 0usize;
    let mut table =
        String::from("| Byte Offset | Width | Field | Type |\n| :--- | :--- | :--- | :--- |\n");

    for f in &s.fields {
        let (start, end) = f.byte_range.ok_or_else(|| {
            format!(
                "E0412 [Layout Error]: field `{}.{}` inside `@layout` struct is missing explicit `@bytes(start..end)` contract",
                s.name, f.name
            )
        })?;
        if start != cursor {
            return Err(format!(
                "E0413 [Layout Error]: struct `{}` has an implicit padding hole or overlap before field `{}`: expected start byte {}, found {}",
                s.name, f.name, cursor, start
            ));
        }
        if end <= start {
            return Err(format!(
                "E0414 [Layout Error]: field `{}.{}` has invalid empty byte range {}..{}",
                s.name, f.name, start, end
            ));
        }
        let width = end - start;
        let expected_field_bytes = type_byte_size(&f.ty, named_sizes).ok_or_else(|| {
            format!(
                "E0417 [Layout Error]: field `{}.{}` has type `{}`, whose byte size is not known, so `@bytes({}..{})` cannot be verified",
                s.name, f.name, f.ty, start, end
            )
        })?;
        if width != expected_field_bytes {
            return Err(format!(
                "E0415 [Layout Error]: field `{}.{}` of type `{}` requires {} bytes, but `@bytes({}..{})` spans {} bytes",
                s.name, f.name, f.ty, expected_field_bytes, start, end, width
            ));
        }
        if !packed {
            let field_align = type_alignment(&f.ty, named_sizes, named_aligns).unwrap_or(1);
            if start % field_align != 0 {
                return Err(format!(
                    "E0418 [Layout Error]: field `{}.{}` of type `{}` needs {}-byte alignment, but starts at byte {}; move it, or mark the struct `@packed`",
                    s.name, f.name, f.ty, field_align, start
                ));
            }
            if field_align > align {
                return Err(format!(
                    "E0419 [Layout Error]: field `{}.{}` of type `{}` needs {}-byte alignment, more than the struct's `align: {}`; raise it, or mark the struct `@packed`",
                    s.name, f.name, f.ty, field_align, align
                ));
            }
        }
        table.push_str(&format!(
            "| `[{}..{})` | `{} B` | `{}` | `{}` |\n",
            start, end, width, f.name, f.ty
        ));
        cursor = end;
    }

    if cursor != expected_size {
        return Err(format!(
            "E0416 [Layout Error]: struct `{}` declared `@layout(size: {})`, but fields end at byte {}",
            s.name, expected_size, cursor
        ));
    }

    Ok(LayoutProof {
        struct_name: s.name.clone(),
        total_bytes: expected_size,
        align_bytes: align,
        markdown_table: table,
    })
}

/// Byte size of a fixed-size field type, or `None` when it is not known.
///
/// Target-dependent types (`usize`, `isize`) have no size here: a `@layout` struct must
/// mean the same bytes on every target.
fn type_byte_size(ty: &str, named_sizes: &HashMap<String, usize>) -> Option<usize> {
    let t = ty.trim();
    if let Some(inner) = t.strip_prefix("sat<").and_then(|s| s.strip_suffix('>')) {
        return type_byte_size(inner, named_sizes);
    }
    if let Some(inner) = t.strip_prefix("wrap<").and_then(|s| s.strip_suffix('>')) {
        return type_byte_size(inner, named_sizes);
    }
    // Fixed array [N]Elem
    if t.starts_with('[') {
        if let Some(close) = t.find(']') {
            let count_str = t[1..close].trim();
            if let Ok(count) = count_str.parse::<usize>() {
                let elem_ty = &t[close + 1..];
                return Some(count * type_byte_size(elem_ty, named_sizes)?);
            }
        }
    }
    match t {
        "bool" | "q0.8" => Some(1),
        "f16" | "bf16" | "q1.15" => Some(2),
        "f32" | "q16.16" => Some(4),
        "f64" => Some(8),
        // A `uN` / `iN` whose width is whole bytes occupies exactly those bytes (`u8` is 1,
        // `u24` is 3); any other width only fits a bitstruct.
        _ => match int_bits(t) {
            Some(bits) if bits % 8 == 0 => Some(bits / 8),
            Some(_) => None,
            None => named_sizes.get(t).copied(),
        },
    }
}

/// Natural alignment of a fixed-size field type: the declared `align` of the module's
/// `@layout` structs, a bitstruct's backing size, an array's element alignment, and otherwise
/// the largest power of two dividing the type's size (`u64` is 8, `u24` is 1), at most 16.
fn type_alignment(
    ty: &str,
    named_sizes: &HashMap<String, usize>,
    named_aligns: &HashMap<String, usize>,
) -> Option<usize> {
    let t = ty.trim();
    if let Some(inner) = t
        .strip_prefix("sat<")
        .or_else(|| t.strip_prefix("wrap<"))
        .and_then(|s| s.strip_suffix('>'))
    {
        return type_alignment(inner, named_sizes, named_aligns);
    }
    if t.starts_with('[') {
        let close = t.find(']')?;
        return type_alignment(&t[close + 1..], named_sizes, named_aligns);
    }
    if let Some(align) = named_aligns.get(t) {
        return Some(*align);
    }
    let size = type_byte_size(t, named_sizes)?;
    Some((1usize << size.trailing_zeros()).min(16))
}

/// Width of an arbitrary-width integer type `uN` / `iN`, `1 <= N <= 128`.
fn int_bits(ty: &str) -> Option<usize> {
    let digits = ty.strip_prefix('u').or_else(|| ty.strip_prefix('i'))?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=128).contains(n))
}

fn backing_bits(backing_type: &str) -> Option<usize> {
    match backing_type {
        "u8" => Some(8),
        "u16" => Some(16),
        "u32" => Some(32),
        "u64" => Some(64),
        "u128" => Some(128),
        _ => None,
    }
}

/// Proves that `bitstruct Name : uN` has contiguous, hole-free `@bits(start..end)` covering `[0..N)`,
/// and that each field's type is exactly as wide as its range.
pub fn verify_bitstruct_layout(
    bs: &BitStructDecl,
    bitstruct_bits: &HashMap<String, usize>,
) -> Result<BitStructProof, String> {
    let total_bits = backing_bits(&bs.backing_type).ok_or_else(|| {
        format!(
            "E0420 [BitStruct Error]: bitstruct `{}` has unsupported backing type `{}`",
            bs.name, bs.backing_type
        )
    })?;

    let mut cursor = 0usize;
    for f in &bs.fields {
        let (start, end) = f.bit_range;
        if start != cursor {
            return Err(format!(
                "E0421 [BitStruct Error]: bitstruct `{}` has bit gap or overlap before `{}`: expected bit {}, got {}",
                bs.name, f.name, cursor, start
            ));
        }
        if end <= start {
            return Err(format!(
                "E0422 [BitStruct Error]: bitstruct `{}.{}` has invalid bit range {}..{}",
                bs.name, f.name, start, end
            ));
        }
        let ty = f.ty.trim();
        let type_bits = if ty == "bool" {
            Some(1)
        } else {
            int_bits(ty).or_else(|| bitstruct_bits.get(ty).copied())
        }
        .ok_or_else(|| {
            format!(
                "E0425 [BitStruct Error]: field `{}.{}` has type `{}`; a bitstruct field must be `bool`, `uN`, `iN` or a bitstruct of this module",
                bs.name, f.name, ty
            )
        })?;
        if type_bits != end - start {
            return Err(format!(
                "E0424 [BitStruct Error]: field `{}.{}` has type `{}`, {} bits wide, but `@bits({}..{})` spans {} bits",
                bs.name, f.name, ty, type_bits, start, end, end - start
            ));
        }
        cursor = end;
    }

    if cursor != total_bits {
        return Err(format!(
            "E0423 [BitStruct Error]: bitstruct `{}` backing `{}` requires {} bits, but fields end at bit {}",
            bs.name, bs.backing_type, total_bits, cursor
        ));
    }

    Ok(BitStructProof {
        name: bs.name.clone(),
        total_bits,
    })
}

/// Verifies `@total` termination obligations and `@branchless` / `@no_alloc` syntactic rules on a function.
pub fn verify_function_contracts(f: &FnDecl) -> Result<(), String> {
    let is_total = f.attrs.iter().any(|a| a.name == "total");
    let is_branchless = f.attrs.iter().any(|a| a.name == "branchless");

    // 0. A returned view may borrow only from a parameter the caller keeps: never an `own` one.
    for source in &f.provenance {
        match f.params.iter().find(|p| &p.name == source) {
            None => {
                return Err(format!(
                    "E0311 [Provenance Error]: `from {}` in the signature of `{}` (line {}) names no parameter of `{}`.",
                    source, f.name, f.line, f.name
                ))
            }
            Some(p) if p.mode == ParamMode::Own => {
                return Err(format!(
                    "E0311 [Provenance Error]: `from {}` in the signature of `{}` (line {}) names an `own` parameter; a returned view cannot borrow from a value the callee consumes.",
                    source, f.name, f.line
                ))
            }
            Some(_) => {}
        }
    }

    // 1. If marked @total and contains a `while` loop, it MUST contain a `decreases` termination measure!
    if is_total {
        let has_while = f.body_tokens.contains(&TokenKind::While);
        let has_decreases =
            !f.decreases.is_empty() || f.body_tokens.contains(&TokenKind::Decreases);
        if has_while && !has_decreases {
            return Err(format!(
                "E0501 [Totality Error]: Function `{}` (line {}) is marked `@total` and contains a `while` loop, but lacks a constructive `decreases <measure>` proof.",
                f.name, f.line
            ));
        }
    }

    // 2. If marked @branchless, it MUST NOT contain unbounded `while` loops or `raise` jumps!
    if is_branchless {
        if f.body_tokens.contains(&TokenKind::While) {
            return Err(format!(
                "E0401 [Hard Micro-Arch Error]: Function `{}` (line {}) is marked `@branchless`, but contains a `while` loop.",
                f.name, f.line
            ));
        }
        if f.body_tokens.contains(&TokenKind::Raise) {
            return Err(format!(
                "E0402 [Hard Micro-Arch Error]: Function `{}` (line {}) is marked `@branchless`, but contains a `raise` control-flow jump.",
                f.name, f.line
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_syntax::parse_module;

    #[test]
    fn test_rejects_struct_with_padding_hole() {
        let bad_code = r#"
module test.bad
@layout(size: 64, align: 64)
pub struct BrokenCell {
  @bytes(0..8)   pub id: u64
  @bytes(12..64) pub rest: [52]u8
}
"#;
        let m = parse_module(bad_code).unwrap();
        let res = verify_module(&m);
        assert!(res.is_err());
        let errs = res.unwrap_err();
        assert!(errs[0].contains("E0413"));
    }

    #[test]
    fn test_layout_field_sized_by_module_bitstruct() {
        let code = r#"
module test.sized
pub bitstruct Tag : u16 {
  @bits(0..16) pub raw: u16
}
@layout(size: 8, align: 8)
pub struct Packet {
  @bytes(0..2) pub tag: Tag
  @bytes(2..8) pub rest: [6]u8
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs.len(), 1);

        let too_wide = code.replace("@bytes(0..2) pub tag", "@bytes(0..4) pub tag");
        let errs = verify_module(&parse_module(&too_wide).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0415"));
    }

    #[test]
    fn test_bitstruct_field_type_must_match_its_range() {
        let code = r#"
module test.bits
pub bitstruct Flags : u8 {
  @bits(0..3) pub mode: u3
  @bits(3..4) pub armed: bool
  @bits(4..8) pub level: i4
}
@layout(size: 4, align: 1)
pub struct Frame {
  @bytes(0..3) pub seq: u24
  @bytes(3..4) pub flags: Flags
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(
            (rep.bitstruct_proofs.len(), rep.layout_proofs.len()),
            (1, 1)
        );

        let narrow = code.replace("pub mode: u3", "pub mode: u2");
        let errs = verify_module(&parse_module(&narrow).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0424"), "{:?}", errs);

        let opaque = code.replace("pub level: i4", "pub level: Level");
        let errs = verify_module(&parse_module(&opaque).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0425"), "{:?}", errs);
    }

    #[test]
    fn test_layout_fields_sit_at_aligned_offsets() {
        let code = r#"
module test.aligned
@layout(size: 16, align: 8)
pub struct Record {
  @bytes(0..4)  pub tag: u32
  @bytes(4..12) pub value: u64
  @bytes(12..16) pub crc: u32
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(
            errs[0].contains("E0418") && errs[0].contains("value"),
            "{:?}",
            errs
        );

        let packed = code.replace(
            "@layout(size: 16, align: 8)",
            "@packed\n@layout(size: 16, align: 8)",
        );
        assert!(verify_module(&parse_module(&packed).unwrap()).is_ok());

        let under_aligned = r#"
module test.under
@layout(size: 8, align: 4)
pub struct Wide {
  @bytes(0..8) pub value: u64
}
"#;
        let errs = verify_module(&parse_module(under_aligned).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0419"), "{:?}", errs);
    }

    #[test]
    fn test_rejects_layout_field_of_unknown_size() {
        let code = r#"
module test.unknown
@layout(size: 16, align: 8)
pub struct Holder {
  @bytes(0..8)  pub len: usize
  @bytes(8..16) pub mystery: Opaque
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0417") && errs[0].contains("usize"));
    }

    #[test]
    fn test_from_clause_must_name_a_kept_parameter() {
        let ok = r#"
module test.views
pub fn head(data: []u8, n: usize) -> []u8 from data
"#;
        assert!(verify_module(&parse_module(ok).unwrap()).is_ok());

        let unknown = ok.replace("from data", "from buffer");
        let errs = verify_module(&parse_module(&unknown).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0311") && errs[0].contains("names no parameter"));

        let consumed = ok.replace("head(data:", "head(own data:");
        let errs = verify_module(&parse_module(&consumed).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0311") && errs[0].contains("`own` parameter"));
    }

    #[test]
    fn test_rejects_branchless_with_while_loop() {
        let bad_branchless = r#"
module test.bad_branchless
@branchless
pub fn bad_loop(x: i32) -> i32 {
  var i = x
  while i > 0 {
    i -= 1
  }
  i
}
"#;
        let m = parse_module(bad_branchless).unwrap();
        let res = verify_module(&m);
        assert!(res.is_err());
        assert!(res.unwrap_err()[0].contains("E0401"));
    }

    #[test]
    fn test_rejects_branchless_with_raise() {
        let code = r#"
module test.jumps
@branchless
pub fn pick(x: i32) -> i32 ! RangeError {
  raise RangeError.TooBig
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(errs[0].contains("E0402"), "{:?}", errs);
        // The same body is accepted without the contract: raise is ordinary control flow.
        let plain = code.replace("@branchless\n", "");
        assert!(verify_module(&parse_module(&plain).unwrap()).is_ok());
    }

    #[test]
    fn test_total_while_without_decreases_is_rejected() {
        let code = r#"
module test.total
@total
pub fn countdown(n: u32) -> u32 {
  var i = n
  while i > 0 {
    i -= 1
  }
  i
}
"#;
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        assert!(
            errs[0].contains("E0501") && errs[0].contains("countdown"),
            "{:?}",
            errs
        );

        // A measure in the signature or in the body satisfies the token check.
        let in_signature = code.replace("-> u32 {", "-> u32\n  decreases n\n{");
        let rep = verify_module(&parse_module(&in_signature).unwrap()).unwrap();
        assert_eq!(rep.total_functions_verified, 1);
        let in_body = code.replace("while i > 0 {", "while i > 0 {\n    decreases i");
        assert!(verify_module(&parse_module(&in_body).unwrap()).is_ok());

        // Without @total, a loop needs no measure; without a loop, @total needs none either.
        let plain = code.replace("@total\n", "");
        assert!(verify_module(&parse_module(&plain).unwrap()).is_ok());
        let no_loop = "module test.total\n@total\npub fn id(n: u32) -> u32 {\n  n\n}\n";
        assert!(verify_module(&parse_module(no_loop).unwrap()).is_ok());
    }

    #[test]
    fn test_fixed_point_fields_have_their_byte_sizes() {
        let code = r#"
module test.fixed
@layout(size: 8, align: 4)
pub struct Sample {
  @bytes(0..4) pub level: q16.16
  @bytes(4..6) pub gain: q1.15
  @bytes(6..7) pub weight: q0.8
  @bytes(7..8) pub pad: u8
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs[0].total_bytes, 8);

        let wrong = code
            .replace(
                "@bytes(0..4) pub level: q16.16",
                "@bytes(0..2) pub level: q16.16",
            )
            .replace("@bytes(4..6) pub gain", "@bytes(2..6) pub gain");
        let errs = verify_module(&parse_module(&wrong).unwrap()).unwrap_err();
        assert!(
            errs[0].contains("E0415") && errs[0].contains("requires 4 bytes"),
            "{:?}",
            errs
        );
    }

    fn first_error(code: &str) -> String {
        let errs = verify_module(&parse_module(code).unwrap()).unwrap_err();
        errs[0].clone()
    }

    #[test]
    fn test_layout_rejects_bad_geometry() {
        let good = "module test.geo\n@layout(size: 8, align: 4)\npub struct Pair {\n  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n}\n";
        let rep = verify_module(&parse_module(good).unwrap()).unwrap();
        assert_eq!(
            (
                rep.layout_proofs[0].total_bytes,
                rep.layout_proofs[0].align_bytes
            ),
            (8, 4)
        );
        assert!(rep.layout_proofs[0]
            .markdown_table
            .contains("| `[4..8)` | `4 B` | `b` | `u32` |"));

        let cases = [
            (good.replace("align: 4", "align: 3"), "E0410"),
            (
                good.replace("size: 8, align: 4", "size: 6, align: 4"),
                "E0411",
            ),
            (good.replace("@bytes(4..8) pub b", "pub b"), "E0412"),
            (
                good.replace("@bytes(0..4) pub a: u32", "@bytes(0..0) pub a: u32")
                    .replace("@bytes(4..8)", "@bytes(0..4)"),
                "E0414",
            ),
            (good.replace("size: 8", "size: 12"), "E0416"),
        ];
        for (code, expected) in cases {
            let err = first_error(&code);
            assert!(err.contains(expected), "expected {}: {}", expected, err);
        }
    }

    #[test]
    fn test_report_counts_contract_functions() {
        let code = r#"
module test.counts
@total
pub fn a(n: u32) -> u32 = n
pub fn b(n: u32) -> u32
  decreases n
= n
@branchless
pub fn c(n: u32) -> u32 = n
@kernel
pub fn d(n: u32) -> u32 = n
@inline
pub fn e(n: u32) -> u32 = n
@example("one")
{
  a(1)
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(
            (
                rep.total_functions_verified,
                rep.branchless_functions_verified,
                rep.kernel_functions_verified,
                rep.examples_count
            ),
            (2, 1, 1, 1)
        );
        assert_eq!(rep.module_path, "test.counts");
    }

    #[test]
    fn test_layout_field_may_be_a_layout_struct() {
        let code = r#"
module test.nested
@layout(size: 8, align: 8)
pub struct Stamp {
  @bytes(0..8) pub nanos: u64
}
@layout(size: 16, align: 8)
pub struct Event {
  @bytes(0..8) pub at: Stamp
  @bytes(8..16) pub value: f64
}
"#;
        let rep = verify_module(&parse_module(code).unwrap()).unwrap();
        assert_eq!(rep.layout_proofs.len(), 2);
        // Stamp's declared alignment applies where it is used.
        let misplaced = code
            .replace("@layout(size: 16, align: 8)\npub struct Event", "@packed\n@layout(size: 20, align: 4)\npub struct Event")
            .replace("@bytes(0..8) pub at: Stamp\n  @bytes(8..16) pub value: f64", "@bytes(0..4) pub tag: u32\n  @bytes(4..12) pub at: Stamp\n  @bytes(12..20) pub value: f64");
        assert!(verify_module(&parse_module(&misplaced).unwrap()).is_ok());
        let unpacked = misplaced.replace("@packed\n", "");
        assert!(first_error(&unpacked).contains("E0418"));
    }

    #[test]
    fn test_field_sizes_and_alignments_follow_their_types() {
        // u12 is not a whole number of bytes, so it belongs in a bitstruct.
        let odd = "module test.sizes\n@layout(size: 2, align: 2)\npub struct Odd {\n  @bytes(0..2) pub v: u12\n}\n";
        assert!(first_error(odd).contains("E0417"));
        // An array is aligned like its element: [2]u32 cannot start at byte 2.
        let arr = "module test.sizes\n@layout(size: 12, align: 4)\npub struct Arr {\n  @bytes(0..2) pub tag: u16\n  @bytes(2..10) pub vals: [2]u32\n  @bytes(10..12) pub crc: u16\n}\n";
        let err = first_error(arr);
        assert!(err.contains("E0418") && err.contains("vals"), "{}", err);
        // Leading zeros do not spell a width.
        let zero = "module test.sizes\npub bitstruct Z : u8 {\n  @bits(0..7) pub v: u07\n  @bits(7..8) pub f: bool\n}\n";
        assert!(first_error(zero).contains("E0425"));
        // Every backing width from u8 to u128 is accepted.
        for (backing, bits) in [
            ("u8", 8),
            ("u16", 16),
            ("u32", 32),
            ("u64", 64),
            ("u128", 128),
        ] {
            let code = format!(
                "module test.sizes\npub bitstruct W : {} {{\n  @bits(0..{}) pub raw: u{}\n}}\n",
                backing, bits, bits
            );
            let rep = verify_module(&parse_module(&code).unwrap()).unwrap();
            assert_eq!(rep.bitstruct_proofs[0].total_bits, bits);
        }
    }

    #[test]
    fn test_bitstruct_rejects_gaps_and_shortfalls() {
        let good = "module test.bits\npub bitstruct Flags : u8 {\n  @bits(0..4) pub lo: u4\n  @bits(4..8) pub hi: u4\n}\n";
        let rep = verify_module(&parse_module(good).unwrap()).unwrap();
        assert_eq!(
            (
                rep.bitstruct_proofs[0].name.as_str(),
                rep.bitstruct_proofs[0].total_bits
            ),
            ("Flags", 8)
        );

        let cases = [
            (good.replace(": u8 {", ": u12 {"), "E0420"),
            (
                good.replace("@bits(4..8) pub hi: u4", "@bits(5..8) pub hi: u3"),
                "E0421",
            ),
            (
                good.replace("@bits(0..4) pub lo: u4", "@bits(0..0) pub lo: u4")
                    .replace("@bits(4..8)", "@bits(0..4)"),
                "E0422",
            ),
            (
                good.replace("@bits(4..8) pub hi: u4", "@bits(4..7) pub hi: u3"),
                "E0423",
            ),
        ];
        for (code, expected) in cases {
            let err = first_error(&code);
            assert!(err.contains(expected), "expected {}: {}", expected, err);
        }
    }
}

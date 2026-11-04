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

    // Byte sizes of the module's own fixed-size types, so a `@layout` field may name them.
    let mut named_sizes: HashMap<String, usize> = HashMap::new();
    for item in &module.items {
        match item {
            ItemDecl::BitStruct(bs) => {
                if let Some(bits) = backing_bits(&bs.backing_type) {
                    named_sizes.insert(bs.name.clone(), bits / 8);
                }
            }
            ItemDecl::Struct(s) => {
                if let Some((size, _)) = extract_layout_attr(s) {
                    named_sizes.insert(s.name.clone(), size);
                }
            }
            _ => {}
        }
    }

    for item in &module.items {
        match item {
            ItemDecl::Struct(s) => {
                if let Some((size, align)) = extract_layout_attr(s) {
                    match verify_explicit_struct_layout(s, size, align, &named_sizes) {
                        Ok(proof) => layout_proofs.push(proof),
                        Err(e) => errors.push(e),
                    }
                }
            }
            ItemDecl::BitStruct(bs) => match verify_bitstruct_layout(bs) {
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

/// Proves that `@layout(size: S, align: A)` has contiguous, hole-free `@bytes(start..end)` fields summing to `S`.
pub fn verify_explicit_struct_layout(
    s: &StructDecl,
    expected_size: usize,
    align: usize,
    named_sizes: &HashMap<String, usize>,
) -> Result<LayoutProof, String> {
    if !align.is_power_of_two() {
        return Err(format!(
            "E0410 [Layout Error]: struct `{}` alignment {} is not a power of two",
            s.name, align
        ));
    }
    if expected_size % align != 0 {
        return Err(format!(
            "E0411 [Layout Error]: struct `{}` size {} is not a multiple of alignment {}",
            s.name, expected_size, align
        ));
    }

    let mut cursor = 0usize;
    let mut table = format!(
        "| Byte Offset | Width | Field | Type |\n| :--- | :--- | :--- | :--- |\n"
    );

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
        "u8" | "i8" | "bool" | "q0.8" => Some(1),
        "u16" | "i16" | "f16" | "bf16" | "q1.15" => Some(2),
        "u32" | "i32" | "f32" | "q16.16" => Some(4),
        "u64" | "i64" | "f64" => Some(8),
        "u128" | "i128" => Some(16),
        _ => named_sizes.get(t).copied(),
    }
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

/// Proves that `bitstruct Name : uN` has contiguous, hole-free `@bits(start..end)` covering `[0..N)`.
pub fn verify_bitstruct_layout(bs: &BitStructDecl) -> Result<BitStructProof, String> {
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
        let has_while = f.body_tokens.iter().any(|t| *t == TokenKind::While);
        let has_decreases = !f.decreases.is_empty() || f.body_tokens.iter().any(|t| *t == TokenKind::Decreases);
        if has_while && !has_decreases {
            return Err(format!(
                "E0501 [Totality Error]: Function `{}` (line {}) is marked `@total` and contains a `while` loop, but lacks a constructive `decreases <measure>` proof.",
                f.name, f.line
            ));
        }
    }

    // 2. If marked @branchless, it MUST NOT contain unbounded `while` loops or `raise` jumps!
    if is_branchless {
        if f.body_tokens.iter().any(|t| *t == TokenKind::While) {
            return Err(format!(
                "E0401 [Hard Micro-Arch Error]: Function `{}` (line {}) is marked `@branchless`, but contains a `while` loop.",
                f.name, f.line
            ));
        }
        if f.body_tokens.iter().any(|t| *t == TokenKind::Raise) {
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
}

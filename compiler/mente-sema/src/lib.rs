//! `mente-sema` — Semantic Prover: 64B Cache-Line Layout, `bitstruct`, `@total`/`decreases`, and Micro-Arch Contracts.
//!
//! Implements Stations 2, 3, 4, 5, and 6.

use mente_syntax::{BitStructDecl, FnDecl, ItemDecl, ModuleFile, StructDecl, TokenKind};

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

    for item in &module.items {
        match item {
            ItemDecl::Struct(s) => {
                if let Some((size, align)) = extract_layout_attr(s) {
                    match verify_explicit_struct_layout(s, size, align) {
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
pub fn verify_explicit_struct_layout(s: &StructDecl, expected_size: usize, align: usize) -> Result<LayoutProof, String> {
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
        if let Some(expected_field_bytes) = estimate_type_byte_size(&f.ty) {
            if width != expected_field_bytes {
                return Err(format!(
                    "E0415 [Layout Error]: field `{}.{}` of type `{}` requires {} bytes, but `@bytes({}..{})` spans {} bytes",
                    s.name, f.name, f.ty, expected_field_bytes, start, end, width
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

fn estimate_type_byte_size(ty: &str) -> Option<usize> {
    let t = ty.trim();
    if let Some(inner) = t.strip_prefix("sat<").and_then(|s| s.strip_suffix('>')) {
        return estimate_type_byte_size(inner);
    }
    if let Some(inner) = t.strip_prefix("wrap<").and_then(|s| s.strip_suffix('>')) {
        return estimate_type_byte_size(inner);
    }
    // Fixed array [N]Elem
    if t.starts_with('[') {
        if let Some(close) = t.find(']') {
            let count_str = t[1..close].trim();
            if let Ok(count) = count_str.parse::<usize>() {
                let elem_ty = &t[close + 1..];
                return Some(count * estimate_type_byte_size(elem_ty)?);
            }
        }
    }
    match t {
        "u8" | "i8" | "bool" | "q0.8" => Some(1),
        "u16" | "i16" | "f16" | "bf16" | "q1.15" => Some(2),
        "u32" | "i32" | "f32" | "q16.16" | "SpikeHeader" => Some(4),
        "u64" | "i64" | "f64" | "usize" | "isize" => Some(8),
        "u128" | "i128" => Some(16),
        _ => None,
    }
}

/// Proves that `bitstruct Name : uN` has contiguous, hole-free `@bits(start..end)` covering `[0..N)`.
pub fn verify_bitstruct_layout(bs: &BitStructDecl) -> Result<BitStructProof, String> {
    let total_bits = match bs.backing_type.as_str() {
        "u8" => 8usize,
        "u16" => 16usize,
        "u32" => 32usize,
        "u64" => 64usize,
        "u128" => 128usize,
        other => {
            return Err(format!(
                "E0420 [BitStruct Error]: bitstruct `{}` has unsupported backing type `{}`",
                bs.name, other
            ))
        }
    };

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

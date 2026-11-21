//! The numbers of `@layout`, `@bytes` and `@bits` (`spec/morphic-layouts.md`): a struct's byte
//! ranges, sizes and alignments, and a bitstruct's bit ranges. Each number comes from the
//! programmer and may be as large as `usize` holds, so no arithmetic here may overflow: every
//! sum, product and difference is checked, and a fix whose numbers would pass `usize` is not
//! offered. Clippy's `arithmetic_side_effects` holds that for every function of this module,
//! one added later included, under the required check `cargo clippy -D warnings`: the fuzzer
//! found three of these overflows one by one (2026-11-14 to 11-17), each fixed where it was.
#![deny(clippy::arithmetic_side_effects)]

use std::cmp::Ordering;
use std::collections::HashMap;

use mente_syntax::diagnostics::{insertion_point, Applicability, Diagnostic};
use mente_syntax::{Attr, BitStructDecl, ItemDecl, ModuleFile, Span, StructDecl, StructFieldDecl};

use crate::structs::Structs;
use crate::{BitStructProof, LayoutProof};

/// The byte sizes and alignments of a module's own fixed-size types, so that a `@layout` field
/// may name them, and the bit widths of its bitstructs, so that a bitstruct field may nest one.
pub(crate) struct Tables {
    pub(crate) named_sizes: HashMap<String, usize>,
    pub(crate) named_aligns: HashMap<String, usize>,
    pub(crate) bitstruct_bits: HashMap<String, usize>,
}

/// The [`Tables`] of `module`, whose declared types are `structs`.
pub(crate) fn tables(module: &ModuleFile, structs: &Structs) -> Tables {
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
                if let Some((size, align, _)) = extract_layout_attr(s) {
                    named_sizes.insert(s.name.clone(), size);
                    named_aligns.insert(s.name.clone(), align);
                }
            }
            ItemDecl::Unit(_)
            | ItemDecl::Enum(_)
            | ItemDecl::Trait(_)
            | ItemDecl::Fn(_)
            | ItemDecl::Example(_) => {}
        }
    }
    for d in &structs.defs {
        if let Some(l) = &d.layout {
            if let (Ok(size), Ok(align)) = (usize::try_from(l.size), usize::try_from(l.align)) {
                named_sizes.entry(d.name.clone()).or_insert(size);
                named_aligns.entry(d.name.clone()).or_insert(align);
            }
        }
    }
    Tables {
        named_sizes,
        named_aligns,
        bitstruct_bits,
    }
}

/// The first `@layout(size: S, align: A)` of a struct that names both, with its span.
pub(crate) fn extract_layout_attr(s: &StructDecl) -> Option<(usize, usize, Span)> {
    for a in s.attrs.iter().filter(|a| a.name == "layout") {
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
            return Some((sz, al, a.span));
        }
    }
    None
}

/// A field's attribute by name.
fn field_attr<'f>(f: &'f StructFieldDecl, name: &str) -> Option<&'f Attr> {
    f.attrs.iter().find(|a| a.name == name)
}

/// Proves that `@layout(size: S, align: A)` has contiguous, hole-free `@bytes(start..end)` fields summing to `S`,
/// each at an offset its type's alignment allows unless the struct is `@packed`.
pub fn verify_explicit_struct_layout(
    s: &StructDecl,
    expected_size: usize,
    align: usize,
    named_sizes: &HashMap<String, usize>,
    named_aligns: &HashMap<String, usize>,
) -> Result<LayoutProof, Diagnostic> {
    let layout = extract_layout_attr(s).map_or(s.name_span, |(_, _, span)| span);
    let layout_text =
        |size: usize, align: usize| format!("@layout(size: {}, align: {})", size, align);
    // A fix is offered only where its numbers exist: the next power of two above an alignment,
    // and the next multiple of it above a size, may lie past `usize`.
    if !align.is_power_of_two() {
        let d = Diagnostic::error(
            "E0410",
            layout,
            format!(
                "struct `{}`: alignment {} is not a power of two",
                s.name, align
            ),
        );
        return Err(match align.checked_next_power_of_two() {
            Some(next) => d.with_fix(
                Applicability::Suggestion,
                format!("align it to {} bytes", next.max(1)),
                layout,
                layout_text(expected_size, next.max(1)),
            ),
            None => d,
        });
    }
    if !expected_size.is_multiple_of(align) {
        let d = Diagnostic::error(
            "E0411",
            layout,
            format!(
                "struct `{}`: size {} is not a multiple of its alignment {}",
                s.name, expected_size, align
            ),
        );
        return Err(match expected_size.div_ceil(align).checked_mul(align) {
            Some(rounded) => d.with_fix(
                Applicability::Suggestion,
                format!("round the size up to {} bytes", rounded),
                layout,
                layout_text(rounded, align),
            ),
            None => d,
        });
    }

    let packed = s.attrs.iter().any(|a| a.name == "packed");
    let mut cursor = 0usize;
    let mut table =
        String::from("| Byte Offset | Width | Field | Type |\n| :--- | :--- | :--- | :--- |\n");

    for f in &s.fields {
        let size = type_byte_size(&f.ty, named_sizes);
        let Some((start, end)) = f.byte_range else {
            let d = Diagnostic::error(
                "E0412",
                f.name_span,
                format!(
                    "field `{}.{}` has no `@bytes(start..end)`; every field of a `@layout` struct needs one",
                    s.name, f.name
                ),
            );
            // The field must start where the last one ended and be as wide as its type, so
            // when the type's size is known exactly one range is possible.
            return Err(match size.and_then(|n| cursor.checked_add(n)) {
                Some(next) => d.with_fix(
                    Applicability::MachineApplicable,
                    format!("give `{}` the bytes {}..{}", f.name, cursor, next),
                    insertion_point(f.span),
                    format!("@bytes({}..{}) ", cursor, next),
                ),
                None => d,
            });
        };
        let bytes = field_attr(f, "bytes").map_or(f.name_span, |a| a.span);
        let what = match start.cmp(&cursor) {
            Ordering::Equal => None,
            Ordering::Greater => Some(format!(
                "a gap before field `{}`: it starts at byte {}, but the previous field ends at byte {}",
                f.name, start, cursor
            )),
            Ordering::Less => Some(format!(
                "field `{}` overlaps the previous field: it starts at byte {}, before byte {} where that field ends",
                f.name, start, cursor
            )),
        };
        if let Some(what) = what {
            let d = Diagnostic::error("E0413", bytes, format!("struct `{}` has {}", s.name, what));
            return Err(match cursor.checked_add(end.saturating_sub(start)) {
                Some(moved) => d.with_fix(
                    Applicability::Suggestion,
                    format!("start `{}` at byte {}", f.name, cursor),
                    bytes,
                    format!("@bytes({}..{})", cursor, moved),
                ),
                None => d,
            });
        }
        let Some(width) = end.checked_sub(start).filter(|&w| w > 0) else {
            let d = Diagnostic::error(
                "E0414",
                bytes,
                format!(
                    "field `{}.{}` has an empty byte range {}..{}",
                    s.name, f.name, start, end
                ),
            );
            return Err(match size.and_then(|n| Some((n, start.checked_add(n)?))) {
                Some((n, wide)) => d.with_fix(
                    Applicability::Suggestion,
                    format!("give `{}` its {} bytes", f.name, n),
                    bytes,
                    format!("@bytes({}..{})", start, wide),
                ),
                None => d,
            });
        };
        let Some(expected_field_bytes) = size else {
            return Err(Diagnostic::error(
                "E0417",
                f.ty_span,
                format!(
                    "field `{}.{}` has type `{}`, whose byte size is not known, so `@bytes({}..{})` cannot be checked",
                    s.name, f.name, f.ty, start, end
                ),
            ));
        };
        if width != expected_field_bytes {
            let d = Diagnostic::error(
                "E0415",
                bytes,
                format!(
                    "field `{}.{}` of type `{}` requires {} bytes, but `@bytes({}..{})` spans {} bytes",
                    s.name, f.name, f.ty, expected_field_bytes, start, end, width
                ),
            );
            return Err(match start.checked_add(expected_field_bytes) {
                Some(wide) => d.with_fix(
                    Applicability::Suggestion,
                    format!("make `{}` {} bytes wide", f.name, expected_field_bytes),
                    bytes,
                    format!("@bytes({}..{})", start, wide),
                ),
                None => d,
            });
        }
        if !packed {
            let field_align = type_alignment(&f.ty, named_sizes, named_aligns).unwrap_or(1);
            if !start.is_multiple_of(field_align) {
                return Err(Diagnostic::error(
                    "E0418",
                    bytes,
                    format!(
                        "field `{}.{}` of type `{}` needs {}-byte alignment, but starts at byte {}",
                        s.name, f.name, f.ty, field_align, start
                    ),
                )
                .with_fix(
                    Applicability::Suggestion,
                    format!(
                        "mark `{}` `@packed`, so its fields may be unaligned",
                        s.name
                    ),
                    insertion_point(layout),
                    "@packed\n",
                ));
            }
            if field_align > align {
                let d = Diagnostic::error(
                    "E0419",
                    f.ty_span,
                    format!(
                        "field `{}.{}` of type `{}` needs {}-byte alignment, more than the struct's `align: {}`",
                        s.name, f.name, f.ty, field_align, align
                    ),
                );
                return Err(if expected_size.is_multiple_of(field_align) {
                    d.with_fix(
                        Applicability::Suggestion,
                        format!("raise `{}`'s alignment to {}", s.name, field_align),
                        layout,
                        layout_text(expected_size, field_align),
                    )
                } else {
                    d
                });
            }
        }
        table.push_str(&format!(
            "| `[{}..{})` | `{} B` | `{}` | `{}` |\n",
            start, end, width, f.name, f.ty
        ));
        cursor = end;
    }

    if cursor != expected_size {
        let d = Diagnostic::error(
            "E0416",
            layout,
            format!(
                "struct `{}` declares `size: {}`, but its fields end at byte {}",
                s.name, expected_size, cursor
            ),
        );
        return Err(if cursor > 0 && cursor.is_multiple_of(align) {
            d.with_fix(
                Applicability::Suggestion,
                format!("declare the size its fields take, {} bytes", cursor),
                layout,
                layout_text(cursor, align),
            )
        } else {
            d
        });
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
/// The type a field's type `ty` holds through its arrays: `E` for `[2][3]E`.
pub(crate) fn element_type(ty: &str) -> &str {
    let mut t = ty.trim();
    while let Some(rest) = t.strip_prefix('[') {
        match rest.split_once(']') {
            Some((_, after)) => t = after.trim(),
            None => break,
        }
    }
    t
}

fn type_byte_size(ty: &str, named_sizes: &HashMap<String, usize>) -> Option<usize> {
    let t = ty.trim();
    if let Some(inner) = t.strip_prefix("sat<").and_then(|s| s.strip_suffix('>')) {
        return type_byte_size(inner, named_sizes);
    }
    if let Some(inner) = t.strip_prefix("wrap<").and_then(|s| s.strip_suffix('>')) {
        return type_byte_size(inner, named_sizes);
    }
    // A fixed array `[N]Elem`: N elements of its element's size, none where that passes `usize`.
    if let Some((count, elem_ty)) = t.strip_prefix('[').and_then(|r| r.split_once(']')) {
        if let Ok(count) = count.trim().parse::<usize>() {
            return count.checked_mul(type_byte_size(elem_ty, named_sizes)?);
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
    if let Some(rest) = t.strip_prefix('[') {
        let (_, elem) = rest.split_once(']')?;
        return type_alignment(elem, named_sizes, named_aligns);
    }
    if let Some(align) = named_aligns.get(t) {
        return Some(*align);
    }
    let size = type_byte_size(t, named_sizes)?;
    // A size of 0 has no power of two dividing it but every one, of which 16 is the most.
    Some(
        1usize
            .checked_shl(size.trailing_zeros())
            .map_or(16, |a| a.min(16)),
    )
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

/// The smallest backing integer that holds `bits` bits.
fn backing_for(bits: usize) -> Option<usize> {
    [8, 16, 32, 64, 128].into_iter().find(|&w| bits <= w)
}

/// Proves that `bitstruct Name : uN` has contiguous, hole-free `@bits(start..end)` covering `[0..N)`,
/// and that each field's type is exactly as wide as its range.
pub fn verify_bitstruct_layout(
    bs: &BitStructDecl,
    bitstruct_bits: &HashMap<String, usize>,
) -> Result<BitStructProof, Diagnostic> {
    let Some(total_bits) = backing_bits(&bs.backing_type) else {
        let d = Diagnostic::error(
            "E0420",
            bs.backing_span,
            format!(
                "bitstruct `{}` has backing type `{}`; it must be `u8`, `u16`, `u32`, `u64` or `u128`",
                bs.name, bs.backing_type
            ),
        );
        return Err(match int_bits(&bs.backing_type).and_then(backing_for) {
            Some(w) => d.with_fix(
                Applicability::Suggestion,
                format!("back it with `u{}`", w),
                bs.backing_span,
                format!("u{}", w),
            ),
            None => d,
        });
    };

    let mut cursor = 0usize;
    for f in &bs.fields {
        let (start, end) = f.bit_range;
        let bits = f.bits_span.unwrap_or(f.name_span);
        let what = match start.cmp(&cursor) {
            Ordering::Equal => None,
            Ordering::Greater => Some("a gap"),
            Ordering::Less => Some("an overlap"),
        };
        if let Some(what) = what {
            let d = Diagnostic::error(
                "E0421",
                bits,
                format!(
                    "bitstruct `{}` has {} before field `{}`: it starts at bit {}, but the previous field ends at bit {}",
                    bs.name, what, f.name, start, cursor
                ),
            );
            return Err(match cursor.checked_add(end.saturating_sub(start)) {
                Some(moved) => d.with_fix(
                    Applicability::Suggestion,
                    format!("start `{}` at bit {}", f.name, cursor),
                    bits,
                    format!("@bits({}..{})", cursor, moved),
                ),
                None => d,
            });
        }
        let Some(width) = end.checked_sub(start).filter(|&w| w > 0) else {
            return Err(Diagnostic::error(
                "E0422",
                bits,
                format!(
                    "field `{}.{}` has an empty bit range {}..{}",
                    bs.name, f.name, start, end
                ),
            ));
        };
        let ty = f.ty.trim();
        let type_bits = if ty == "bool" {
            Some(1)
        } else {
            int_bits(ty).or_else(|| bitstruct_bits.get(ty).copied())
        };
        let Some(type_bits) = type_bits else {
            return Err(Diagnostic::error(
                "E0425",
                f.ty_span,
                format!(
                    "field `{}.{}` has type `{}`; a bitstruct field must be `bool`, `uN`, `iN` or a bitstruct of this module",
                    bs.name, f.name, ty
                ),
            ));
        };

        if type_bits != width {
            let d = Diagnostic::error(
                "E0424",
                f.ty_span,
                format!(
                    "field `{}.{}` has type `{}`, {} bits wide, but `@bits({}..{})` spans {} bits",
                    bs.name, f.name, ty, type_bits, start, end, width
                ),
            );
            // An integer field can take the width of its range; a nested bitstruct cannot.
            let fitted = if ty.starts_with('i') && int_bits(ty).is_some() {
                Some(format!("i{}", width))
            } else if ty == "bool" || int_bits(ty).is_some() {
                Some(if width == 1 {
                    String::from("bool")
                } else {
                    format!("u{}", width)
                })
            } else {
                None
            };
            return Err(match fitted {
                Some(t) => d.with_fix(
                    Applicability::Suggestion,
                    format!("make `{}` a `{}`", f.name, t),
                    f.ty_span,
                    t,
                ),
                None => d,
            });
        }
        cursor = end;
    }

    if cursor != total_bits {
        let d = Diagnostic::error(
            "E0423",
            bs.backing_span,
            format!(
                "bitstruct `{}`: backing `{}` has {} bits, but its fields end at bit {}",
                bs.name, bs.backing_type, total_bits, cursor
            ),
        );
        return Err(match backing_bits(&format!("u{}", cursor)) {
            Some(w) => d.with_fix(
                Applicability::Suggestion,
                format!("back it with `u{}`, as wide as its fields", w),
                bs.backing_span,
                format!("u{}", w),
            ),
            None => d,
        });
    }

    Ok(BitStructProof {
        name: bs.name.clone(),
        total_bits,
    })
}

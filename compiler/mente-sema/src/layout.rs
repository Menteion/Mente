//! The numbers of `@layout`, `@bytes` and `@bits` (`spec/morphic-layouts.md`, item 2): a struct's
//! byte ranges, sizes and alignments, and a bitstruct's bit ranges. Each number comes from the
//! programmer, read as every integer literal is ([`mente_syntax::int_literal_value`]), and is a
//! [`LayoutNum`] here: at most the bytes a value may hold ([`VALUE_BYTES`], `E0223`), held alike
//! on every host, with no arithmetic but its checked methods, which give none past that. A number
//! past it is an error that says so, and a fix whose numbers would pass it is not offered.
//! Clippy's `arithmetic_side_effects` is forbidden in this module, a function added later
//! included, under the required check `cargo clippy -D warnings`: the fuzzer found three
//! overflows here one by one (2026-11-14 to 11-17), and the review of pull request 187 numbers
//! past `usize` that were never read.
#![forbid(clippy::arithmetic_side_effects)]

use std::cmp::Ordering;
use std::collections::HashMap;

use mente_syntax::diagnostics::{insertion_point, Applicability, Diagnostic};
use mente_syntax::{
    int_literal, int_literal_value, Attr, BitStructDecl, ItemDecl, ModuleFile, Span, StructDecl,
    StructFieldDecl,
};

use crate::structs::{Structs, ARRAY_ELEMENTS, VALUE_BYTES};
use crate::{BitStructProof, LayoutProof};

/// The widest backing a bitstruct may have, `u128`, in bits: the most a bit number may be.
const BACKING_BITS: u32 = 128;

/// [`LayoutNum`] alone, in a module of its own: its field and its constructor are private to it,
/// so no code of this file outside it reads a raw value or builds an unchecked one, and a new
/// operation is one more checked method here. The compiler's privacy holds that; the workspace
/// runs no doctests (the required check "No doctests"), so no compile-fail test states it.
mod num {
    use std::fmt;

    use crate::structs::VALUE_BYTES;

    /// A number of `@layout`, `@bytes` or `@bits`, an array's length in a field's type, or a size,
    /// an offset or an alignment computed from them: at most [`VALUE_BYTES`], held the same on every
    /// host. It has no arithmetic operators and gives no raw value: each sum, difference, product and
    /// power is a checked method, whose `None` is a result past [`VALUE_BYTES`], so that no number
    /// a programmer writes wraps, on any host, and a new operation can only be one more checked
    /// method here.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct LayoutNum(u128);

    impl LayoutNum {
        pub(crate) const ZERO: LayoutNum = LayoutNum(0);

        /// A number no larger than `u32` holds, which every layout number may be.
        pub(crate) const fn small(n: u32) -> LayoutNum {
            LayoutNum(n as u128)
        }

        /// `n`, where it is no more than [`VALUE_BYTES`].
        pub(crate) fn within(n: u128) -> Option<LayoutNum> {
            (n <= VALUE_BYTES).then_some(LayoutNum(n))
        }

        pub(crate) fn checked_add(self, o: LayoutNum) -> Option<LayoutNum> {
            LayoutNum::within(self.0.checked_add(o.0)?)
        }

        pub(crate) fn checked_sub(self, o: LayoutNum) -> Option<LayoutNum> {
            self.0.checked_sub(o.0).map(LayoutNum)
        }

        pub(crate) fn checked_mul(self, o: LayoutNum) -> Option<LayoutNum> {
            LayoutNum::within(self.0.checked_mul(o.0)?)
        }

        pub(crate) fn is_power_of_two(self) -> bool {
            self.0.is_power_of_two()
        }

        /// The least power of two at least this, and at least 1.
        pub(crate) fn next_power_of_two(self) -> Option<LayoutNum> {
            LayoutNum::within(self.0.max(1).checked_next_power_of_two()?)
        }

        /// Whether this is a multiple of `o`, which 0 is of every number and only 0 of 0.
        pub(crate) fn is_multiple_of(self, o: LayoutNum) -> bool {
            self.0.is_multiple_of(o.0)
        }

        /// The least multiple of `o` at least this, for an `o` above 0.
        pub(crate) fn round_up_to(self, o: LayoutNum) -> Option<LayoutNum> {
            LayoutNum(self.0.checked_div(o.0)?)
                .checked_add(LayoutNum::small(u32::from(!self.is_multiple_of(o))))?
                .checked_mul(o)
        }

        /// The largest power of two that divides this, at most 16; 16 for 0, which every power
        /// divides.
        pub(crate) fn natural_alignment(self) -> LayoutNum {
            let most = LayoutNum::small(16);
            1u128
                .checked_shl(self.0.trailing_zeros())
                .map_or(most, |a| LayoutNum(a).min(most))
        }

        /// The bytes of a whole-byte width in bits, `None` for any other.
        pub(super) fn bytes_of_bits(self) -> Option<LayoutNum> {
            self.0
                .is_multiple_of(8)
                .then(|| LayoutNum(self.0.checked_div(8).unwrap_or(0)))
        }
    }

    impl fmt::Display for LayoutNum {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }
}

pub use num::LayoutNum;

/// `E0223` for a layout number past [`VALUE_BYTES`], which `what` names with where it stands.
fn past_value_bytes(span: Span, what: String) -> Diagnostic {
    Diagnostic::error(
        "E0223",
        span,
        format!(
            "{}, more than the {} bytes a value, kept in one piece, may hold; a `@layout` struct is a value",
            what, VALUE_BYTES
        ),
    )
}

/// `E0427` where `attrs` hold a second attribute named `name`: a layout states each of its
/// numbers once, and every reader takes the first, so a second is an error that names it rather
/// than numbers no check reads (`morphic-layouts.md`, item 2). `whose` names what holds it.
fn written_twice(attrs: &[Attr], name: &str, whose: &str) -> Option<Diagnostic> {
    let second = attrs.iter().filter(|a| a.name == name).nth(1)?;
    Some(Diagnostic::error(
        "E0427",
        second.span,
        format!(
            "{} has a second `@{}`, `{}`; a layout states each of its numbers once, so keep one",
            whose, name, second
        ),
    ))
}

/// Why the first of `texts` that is an integer literal with a suffix is no layout number: a
/// layout number counts bytes, bits or elements, which have no type, so it takes no suffix
/// ([`int_literal_value`]). `None` where none has one.
fn suffixed<'t>(texts: impl IntoIterator<Item = &'t str>) -> Option<String> {
    texts.into_iter().map(str::trim).find_map(|t| {
        let (n, suffix) = int_literal(t)?;
        suffix.map(|s| {
            format!(
                "`{}` has the suffix `{}`, and a layout number counts bytes, bits or elements, which have no type: write `{}`",
                t, s, n
            )
        })
    })
}

/// The byte sizes and alignments of a module's own fixed-size types, so that a `@layout` field
/// may name them, and the bit widths of its bitstructs, so that a bitstruct field may nest one.
pub(crate) struct Tables {
    pub(crate) named_sizes: HashMap<String, LayoutNum>,
    pub(crate) named_aligns: HashMap<String, LayoutNum>,
    pub(crate) bitstruct_bits: HashMap<String, LayoutNum>,
}

/// The [`Tables`] of `module`, whose declared types are `structs`.
pub(crate) fn tables(module: &ModuleFile, structs: &Structs) -> Tables {
    let mut named_sizes: HashMap<String, LayoutNum> = HashMap::new();
    let mut named_aligns: HashMap<String, LayoutNum> = HashMap::new();
    let mut bitstruct_bits: HashMap<String, LayoutNum> = HashMap::new();
    for item in &module.items {
        match item {
            ItemDecl::BitStruct(bs) => {
                if let Some(bits) = backing_bits(&bs.backing_type) {
                    let bytes = bits.bytes_of_bits().unwrap_or(LayoutNum::ZERO);
                    named_sizes.insert(bs.name.clone(), bytes);
                    named_aligns.insert(bs.name.clone(), bytes);
                    bitstruct_bits.insert(bs.name.clone(), bits);
                }
            }
            ItemDecl::Struct(s) => {
                if let Some(Ok((size, align, _))) = layout_attr(s) {
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
            if let (Some(size), Some(align)) =
                (LayoutNum::within(l.size), LayoutNum::within(l.align))
            {
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

/// What a struct's first `@layout` says: nothing without one; its size, its alignment and its
/// span; or the error of a second `@layout` (`E0427`), of one whose `size` or `align` is
/// missing or no integer literal without a suffix (`E0426`), or past the bytes a value may hold
/// (`E0223`).
pub(crate) fn layout_attr(
    s: &StructDecl,
) -> Option<Result<(LayoutNum, LayoutNum, Span), Diagnostic>> {
    let a = s.attrs.iter().find(|a| a.name == "layout")?;
    if let Some(d) = written_twice(&s.attrs, "layout", &format!("struct `{}`", s.name)) {
        return Some(Err(d));
    }
    let malformed = |what: String| {
        Diagnostic::error(
            "E0426",
            a.span,
            format!(
                "the `@layout` of struct `{}` {}; it is written `@layout(size: S, align: A)`, each an integer literal",
                s.name, what
            ),
        )
    };
    let (mut size, mut align) = (None, None);
    for part in a.args.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once(':').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("size", v)) => size = Some(v),
            Some(("align", v)) => align = Some(v),
            Some(_) | None => {
                return Some(Err(malformed(format!(
                    "has `{}`, which is neither `size` nor `align`",
                    part
                ))))
            }
        }
    }
    let read = |name: &str, v: Option<&str>| -> Result<LayoutNum, Diagnostic> {
        let v = v.ok_or_else(|| malformed(format!("gives no `{}`", name)))?;
        let n = int_literal_value(v).ok_or_else(|| {
            malformed(match suffixed([v]) {
                Some(why) => format!("gives `{}: {}`, but {}", name, v, why),
                None => format!("gives `{}: {}`, which is no integer literal", name, v),
            })
        })?;
        LayoutNum::within(n).ok_or_else(|| {
            past_value_bytes(
                a.span,
                format!("struct `{}` declares `{}: {}`", s.name, name, n),
            )
        })
    };
    Some(read("size", size).and_then(|sz| Ok((sz, read("align", align)?, a.span))))
}

/// A field's attribute by name.
fn field_attr<'f>(f: &'f StructFieldDecl, name: &str) -> Option<&'f Attr> {
    f.attrs.iter().find(|a| a.name == name)
}

/// Proves that `@layout(size: S, align: A)` has contiguous, hole-free `@bytes(start..end)` fields summing to `S`,
/// each at an offset its type's alignment allows unless the struct is `@packed`.
pub fn verify_explicit_struct_layout(
    s: &StructDecl,
    expected_size: LayoutNum,
    align: LayoutNum,
    named_sizes: &HashMap<String, LayoutNum>,
    named_aligns: &HashMap<String, LayoutNum>,
) -> Result<LayoutProof, Diagnostic> {
    let layout = s
        .attrs
        .iter()
        .find(|a| a.name == "layout")
        .map_or(s.name_span, |a| a.span);
    let layout_text =
        |size: LayoutNum, align: LayoutNum| format!("@layout(size: {}, align: {})", size, align);
    // A fix is offered only where its numbers exist: the next power of two above an alignment,
    // and the next multiple of it above a size, may lie past what a value may hold.
    if !align.is_power_of_two() {
        let d = Diagnostic::error(
            "E0410",
            layout,
            format!(
                "struct `{}`: alignment {} is not a power of two",
                s.name, align
            ),
        );
        return Err(match align.next_power_of_two() {
            Some(next) => d.with_fix(
                Applicability::Suggestion,
                format!("align it to {} bytes", next),
                layout,
                layout_text(expected_size, next),
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
        return Err(match expected_size.round_up_to(align) {
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
    let mut cursor = LayoutNum::ZERO;
    let mut table =
        String::from("| Byte Offset | Width | Field | Type |\n| :--- | :--- | :--- | :--- |\n");

    for f in &s.fields {
        let whose = format!("field `{}.{}`", s.name, f.name);
        if let Some(d) = written_twice(&f.attrs, "bytes", &whose) {
            return Err(d);
        }
        let size = type_byte_size(&f.ty, named_sizes);
        let known = size.ok();
        let written = field_attr(f, "bytes");
        let Some((start, end)) = f.byte_range else {
            let d = Diagnostic::error(
                "E0412",
                f.name_span,
                match written {
                    Some(a) => match suffixed(a.args.split("..")) {
                        Some(why) => format!(
                            "{} has `{}`, but {}; every field of a `@layout` struct needs a range `start..end` of integer literals",
                            whose, a, why
                        ),
                        None => format!(
                            "{} has `{}`, which is no range `start..end` of integer literals; every field of a `@layout` struct needs one",
                            whose, a
                        ),
                    },
                    None => format!(
                        "field `{}.{}` has no `@bytes(start..end)`; every field of a `@layout` struct needs one",
                        s.name, f.name
                    ),
                },
            );
            // The field must start where the last one ended and be as wide as its type, so
            // when the type's size is known exactly one range is possible. It replaces a
            // `@bytes` that is written but cannot be read.
            return Err(match known.and_then(|n| cursor.checked_add(n)) {
                Some(next) => {
                    let range = format!("@bytes({}..{})", cursor, next);
                    let message = format!("give `{}` the bytes {}..{}", f.name, cursor, next);
                    match written {
                        Some(a) => {
                            d.with_fix(Applicability::MachineApplicable, message, a.span, range)
                        }
                        None => d.with_fix(
                            Applicability::MachineApplicable,
                            message,
                            insertion_point(f.span),
                            format!("{} ", range),
                        ),
                    }
                }
                None => d,
            });
        };
        let bytes = written.map_or(f.name_span, |a| a.span);
        let (Some(start), Some(end)) = (LayoutNum::within(start), LayoutNum::within(end)) else {
            return Err(past_value_bytes(
                bytes,
                format!(
                    "field `{}.{}` has `@bytes({}..{})`, which reaches byte {}",
                    s.name,
                    f.name,
                    start,
                    end,
                    start.max(end)
                ),
            ));
        };
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
            let width = end.checked_sub(start).unwrap_or(LayoutNum::ZERO);
            return Err(match cursor.checked_add(width) {
                Some(moved) => d.with_fix(
                    Applicability::Suggestion,
                    format!("start `{}` at byte {}", f.name, cursor),
                    bytes,
                    format!("@bytes({}..{})", cursor, moved),
                ),
                None => d,
            });
        }
        let Some(width) = end.checked_sub(start).filter(|&w| w > LayoutNum::ZERO) else {
            let d = Diagnostic::error(
                "E0414",
                bytes,
                format!(
                    "field `{}.{}` has an empty byte range {}..{}",
                    s.name, f.name, start, end
                ),
            );
            return Err(match known.and_then(|n| Some((n, start.checked_add(n)?))) {
                Some((n, wide)) => d.with_fix(
                    Applicability::Suggestion,
                    format!("give `{}` its {} bytes", f.name, n),
                    bytes,
                    format!("@bytes({}..{})", start, wide),
                ),
                None => d,
            });
        };
        let expected_field_bytes = match size {
            Ok(n) => n,
            Err(NoSize::Unknown) => {
                // An array's length is a layout number too, and takes no suffix.
                let lengths = f.ty.split('[').skip(1).filter_map(|p| p.split(']').next());
                return Err(Diagnostic::error(
                    "E0417",
                    f.ty_span,
                    match suffixed(lengths) {
                        Some(why) => format!(
                            "{} has type `{}`, whose byte size is not known: {}",
                            whose, f.ty, why
                        ),
                        None => format!(
                            "{} has type `{}`, whose byte size is not known, so `@bytes({}..{})` cannot be checked",
                            whose, f.ty, start, end
                        ),
                    },
                ))
            }
            Err(NoSize::PastLimit) => {
                return Err(Diagnostic::error(
                    "E0417",
                    f.ty_span,
                    format!(
                        "field `{}.{}` has type `{}`, which holds more than the {} bytes, or the {} elements, a value may hold on every target, so it has no size to check; keep fewer elements, or split the field",
                        s.name, f.name, f.ty, VALUE_BYTES, ARRAY_ELEMENTS
                    ),
                ))
            }
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
            let field_align =
                type_alignment(&f.ty, named_sizes, named_aligns).unwrap_or(LayoutNum::small(1));
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
        return Err(
            if cursor > LayoutNum::ZERO && cursor.is_multiple_of(align) {
                d.with_fix(
                    Applicability::Suggestion,
                    format!("declare the size its fields take, {} bytes", cursor),
                    layout,
                    layout_text(cursor, align),
                )
            } else {
                d
            },
        );
    }

    Ok(LayoutProof {
        struct_name: s.name.clone(),
        total_bytes: expected_size,
        align_bytes: align,
        markdown_table: table,
    })
}

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

/// Why a field's type has no byte size here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoSize {
    /// It is no fixed-size type of the module or the language.
    Unknown,
    /// It is an array of more elements, or more bytes, than a value may hold.
    PastLimit,
}

/// Byte size of a fixed-size field type.
///
/// Target-dependent types (`usize`, `isize`) have no size here: a `@layout` struct must mean the
/// same bytes on every target. An array's length is read as any integer literal is.
fn type_byte_size(ty: &str, named_sizes: &HashMap<String, LayoutNum>) -> Result<LayoutNum, NoSize> {
    let t = ty.trim();
    if let Some(inner) = t
        .strip_prefix("sat<")
        .or_else(|| t.strip_prefix("wrap<"))
        .and_then(|s| s.strip_suffix('>'))
    {
        return type_byte_size(inner, named_sizes);
    }
    // A fixed array `[N]Elem`: N elements of its element's size.
    if let Some((count, elem_ty)) = t.strip_prefix('[').and_then(|r| r.split_once(']')) {
        let n = int_literal_value(count.trim()).ok_or(NoSize::Unknown)?;
        let elem = type_byte_size(elem_ty, named_sizes)?;
        if n > ARRAY_ELEMENTS {
            return Err(NoSize::PastLimit);
        }
        let n = LayoutNum::within(n).ok_or(NoSize::PastLimit)?;
        return n.checked_mul(elem).ok_or(NoSize::PastLimit);
    }
    let small = |n: u32| Ok(LayoutNum::small(n));
    match t {
        "bool" | "q0.8" => small(1),
        "f16" | "bf16" | "q1.15" => small(2),
        "f32" | "q16.16" => small(4),
        "f64" => small(8),
        // A `uN` / `iN` whose width is whole bytes occupies exactly those bytes (`u8` is 1,
        // `u24` is 3); any other width only fits a bitstruct.
        _ => match int_bits(t) {
            Some(bits) => bits.bytes_of_bits().ok_or(NoSize::Unknown),
            None => named_sizes.get(t).copied().ok_or(NoSize::Unknown),
        },
    }
}

/// Natural alignment of a fixed-size field type: the declared `align` of the module's
/// `@layout` structs, a bitstruct's backing size, an array's element alignment, and otherwise
/// the largest power of two dividing the type's size (`u64` is 8, `u24` is 1), at most 16.
fn type_alignment(
    ty: &str,
    named_sizes: &HashMap<String, LayoutNum>,
    named_aligns: &HashMap<String, LayoutNum>,
) -> Option<LayoutNum> {
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
    type_byte_size(t, named_sizes)
        .ok()
        .map(LayoutNum::natural_alignment)
}

/// Width of an arbitrary-width integer type `uN` / `iN`, `1 <= N <= 128`.
fn int_bits(ty: &str) -> Option<LayoutNum> {
    let digits = ty.strip_prefix('u').or_else(|| ty.strip_prefix('i'))?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=BACKING_BITS).contains(n))
        .map(LayoutNum::small)
}

fn backing_bits(backing_type: &str) -> Option<LayoutNum> {
    match backing_type {
        "u8" => Some(LayoutNum::small(8)),
        "u16" => Some(LayoutNum::small(16)),
        "u32" => Some(LayoutNum::small(32)),
        "u64" => Some(LayoutNum::small(64)),
        "u128" => Some(LayoutNum::small(BACKING_BITS)),
        _ => None,
    }
}

/// The smallest backing integer that holds `bits` bits.
fn backing_for(bits: LayoutNum) -> Option<LayoutNum> {
    [8, 16, 32, 64, BACKING_BITS]
        .into_iter()
        .map(LayoutNum::small)
        .find(|&w| bits <= w)
}

/// Proves that `bitstruct Name : uN` has contiguous, hole-free `@bits(start..end)` covering `[0..N)`,
/// and that each field's type is exactly as wide as its range.
pub fn verify_bitstruct_layout(
    bs: &BitStructDecl,
    bitstruct_bits: &HashMap<String, LayoutNum>,
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

    let widest = LayoutNum::small(BACKING_BITS);
    let mut cursor = LayoutNum::ZERO;
    for f in &bs.fields {
        if let Some(d) = written_twice(&f.attrs, "bits", &format!("field `{}.{}`", bs.name, f.name))
        {
            return Err(d);
        }
        let bits = f.bits_span.unwrap_or(f.name_span);
        let first = f.attrs.iter().find(|a| a.name == "bits");
        let Some((start, end)) = f.bit_range else {
            return Err(Diagnostic::error(
                "E0426",
                bits,
                match (first, first.and_then(|a| suffixed(a.args.split("..")))) {
                    (Some(_), Some(why)) => format!(
                        "field `{}.{}` has a `@bits` that is no range `start..end` of integer literals: {}",
                        bs.name, f.name, why
                    ),
                    (Some(_), None) => format!(
                        "field `{}.{}` has a `@bits` that is no range `start..end` of integer literals",
                        bs.name, f.name
                    ),
                    (None, _) => format!(
                        "field `{}.{}` has no `@bits(start..end)`; every field of a bitstruct needs one",
                        bs.name, f.name
                    ),
                },
            ));
        };
        let (Some(start), Some(end)) = (
            LayoutNum::within(start).filter(|&n| n <= widest),
            LayoutNum::within(end).filter(|&n| n <= widest),
        ) else {
            return Err(Diagnostic::error(
                "E0423",
                bits,
                format!(
                    "field `{}.{}` reaches bit {}, past bit {}, where the widest backing, `u128`, ends",
                    bs.name,
                    f.name,
                    start.max(end),
                    BACKING_BITS
                ),
            ));
        };
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
            let width = end.checked_sub(start).unwrap_or(LayoutNum::ZERO);
            return Err(match cursor.checked_add(width).filter(|&m| m <= widest) {
                Some(moved) => d.with_fix(
                    Applicability::Suggestion,
                    format!("start `{}` at bit {}", f.name, cursor),
                    bits,
                    format!("@bits({}..{})", cursor, moved),
                ),
                None => d,
            });
        }
        let Some(width) = end.checked_sub(start).filter(|&w| w > LayoutNum::ZERO) else {
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
            Some(LayoutNum::small(1))
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
                Some(if width == LayoutNum::small(1) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{type_byte_size, LayoutNum, NoSize};
    use crate::structs::ARRAY_ELEMENTS;

    /// An array of the most elements an array may hold has a size, its elements' size times
    /// their count, where that stays within the bytes a value may hold; one element more is past
    /// the limit, whatever its elements. The two limits are equal, so only an array at the limit
    /// tells the check of the elements from the check of the bytes after it (the mutants of pull
    /// request 187 that turned `>` into `>=` and `==` there survived every test).
    #[test]
    fn test_an_array_of_the_most_elements_has_a_size() {
        let none = HashMap::new();
        let most = format!("[{}]u8", ARRAY_ELEMENTS);
        assert_eq!(
            type_byte_size(&most, &none).ok(),
            LayoutNum::within(ARRAY_ELEMENTS)
        );
        let more = format!("[{}]u8", ARRAY_ELEMENTS + 1);
        assert_eq!(type_byte_size(&more, &none).err(), Some(NoSize::PastLimit));
    }
}

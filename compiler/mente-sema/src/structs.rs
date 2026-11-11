//! M2's structs (`spec/core-calculus.md`, items 13 and 21).
//!
//! [`declare`] reads the structs of a module that M2's calculus has, every one without
//! `@layout`, checks their declarations, and gives each one that checks its layout. Every later
//! phase reads a struct's fields and layout from the [`Structs`] it returns: the type checker,
//! Tier 1, the reference interpreter and the lowering to MIR, so that each fact about a struct
//! is defined once.
//!
//! A declaration's checks: each field's type is one M2 has, a struct of the module included
//! (`E0116` for a name that is none, `E0109` for a type whose values a later milestone brings);
//! a refinement's bound is a literal, since no binding is visible in a declaration (`E0201`); a
//! name is declared once in a module, none named like a type of the language, and a field once
//! in a struct (`E0228`); no struct contains
//! itself (`E0225`); and a struct's value fits in one piece (`E0223`).

use std::collections::{HashMap, HashSet};

use mente_syntax::diagnostics::Diagnostic;
use mente_syntax::tree::{Bound, StructSyntax, Tree, TypeId, TypeKind};
use mente_syntax::{ItemDecl, ModuleFile, ParsedStruct, Span};

use crate::names::closest;
use crate::types::{Int, IntTy, Ty, INT_TYPES};

/// The most bytes a value may hold: it is kept in one piece, in one stack slot, whose size
/// native code holds in 32 bits (`core-calculus.md`, items 3 and 21).
pub const VALUE_BYTES: u128 = u32::MAX as u128;

/// The most elements an array may hold, whatever their size: native code counts them, as it
/// does bytes, in 32 bits (`core-calculus.md`, item 3).
pub const ARRAY_ELEMENTS: u128 = u32::MAX as u128;

/// `E0223` for an array type, written `text` at `span`, of `len` elements, more than
/// [`ARRAY_ELEMENTS`].
pub fn too_many_elements(span: Span, text: &str, len: u128) -> Diagnostic {
    Diagnostic::error(
        "E0223",
        span,
        format!(
            "an array of type `{}` holds {} elements, more than the {} an array may hold, whatever their size",
            text, len, ARRAY_ELEMENTS
        ),
    )
}

/// Where a struct's fields lie in its value (`core-calculus.md`, item 21): its size and
/// alignment in bytes, and each field's offset, in the order of the declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: u128,
    pub align: u128,
    pub offsets: Vec<u128>,
}

/// One field of a struct: its name, its type, and its declared type in the struct's tree, which
/// holds its refinement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub decl: TypeId,
}

/// One struct of the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
    pub syntax: StructSyntax,
    pub fields: Vec<FieldDef>,
    /// Whether its declaration parsed and checked without an error.
    pub ok: bool,
    /// Its layout, when its declaration checks.
    pub layout: Option<Layout>,
}

impl StructDef {
    /// The tree its fields' declared types live in.
    pub fn tree(&self) -> &Tree {
        &self.syntax.tree
    }

    /// The field named `name`, with its index in the declaration.
    pub fn field(&self, name: &str) -> Option<(usize, &FieldDef)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

/// The structs of a module, and the names of its other type declarations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Structs {
    pub defs: Vec<StructDef>,
    /// Each type the module declares whose values M2 does not have.
    pub later: HashMap<String, Later>,
}

/// A type a module declares whose values a later milestone brings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Later {
    /// What the declaration is: "a struct", "a bitstruct", "an enum" or "a trait".
    pub kind: &'static str,
    /// What a use of it as a value or a type would be.
    pub what: &'static str,
    /// The milestone that brings it.
    pub milestone: &'static str,
}

impl Later {
    /// `E0109` for a use of the type named `n` at `span`.
    pub fn diagnostic(&self, n: &str, span: Span) -> Diagnostic {
        Diagnostic::error(
            "E0109",
            span,
            format!(
                "{} (`{}`) comes in {}; M1 does not have it",
                self.what, n, self.milestone
            ),
        )
    }
}

impl Structs {
    pub fn get(&self, name: &str) -> Option<&StructDef> {
        self.defs.iter().find(|d| d.name == name)
    }

    /// The size and alignment in bytes of a value of `t` (`core-calculus.md`, item 21): an
    /// integer takes its width and is aligned to it, `usize` 8 bytes, `bool` 1 byte, `[N]T` `N`
    /// times `T`'s size, aligned as `T`, and a struct its layout's. An integer of no one type
    /// counts sixteen bytes, the widest machine type lowering may keep it in. `None` for a type
    /// without a layout: no value, any type, or a struct whose declaration does not check.
    pub fn layout_of(&self, t: &Ty) -> Option<(u128, u128)> {
        match t {
            Ty::Int(Int::One(i) | Int::Mixed(Some(i))) => {
                let bytes = u128::from(i.bits / 8);
                Some((bytes, bytes))
            }
            Ty::Int(Int::Literal | Int::Mixed(None)) => Some((16, 16)),
            Ty::Bool => Some((1, 1)),
            Ty::Array(n, elem) => {
                let (size, align) = self.layout_of(elem)?;
                Some((n.saturating_mul(size), align))
            }
            Ty::Struct(name) => self.get(name)?.layout.as_ref().map(|l| (l.size, l.align)),
            Ty::Unit | Ty::Any => None,
        }
    }
}

/// Places fields of the given sizes and alignments (`core-calculus.md`, item 21): by
/// alignment, the largest first, fields of equal alignment in the order of the declaration,
/// each at the next multiple of its alignment, and the size rounded up to the largest
/// alignment. The offsets come back in the order of the declaration.
pub fn place_fields(fields: &[(u128, u128)]) -> Layout {
    let mut order: Vec<usize> = (0..fields.len()).collect();
    // A stable sort keeps fields of equal alignment in the order of the declaration.
    order.sort_by(|&a, &b| fields[b].1.cmp(&fields[a].1));
    let align = fields.iter().map(|f| f.1).max().unwrap_or(1).max(1);
    let mut offsets = vec![0u128; fields.len()];
    let mut at = 0u128;
    for i in order {
        let (size, a) = fields[i];
        at = at.saturating_add(a - 1) / a * a;
        offsets[i] = at;
        at = at.saturating_add(size);
    }
    let size = at.saturating_add(align - 1) / align * align;
    Layout {
        size,
        align,
        offsets,
    }
}

/// Reads and checks the structs of `module` that `parsed` holds, and the names its items
/// declare. The diagnostics come in the order of the module.
pub fn declare(module: &ModuleFile, parsed: &[ParsedStruct]) -> (Structs, Vec<Diagnostic>) {
    let mut later = HashMap::new();
    let mut add = |name: &str, kind, what, milestone| {
        later.insert(
            name.to_string(),
            Later {
                kind,
                what,
                milestone,
            },
        );
    };
    // A name is declared once in a module: a function, a struct of any kind or a bitstruct.
    let mut items: Vec<(&str, Span, &'static str)> = Vec::new();
    // And no struct or bitstruct takes the name of a type of the language.
    let mut reserved: Vec<(&str, Span, &'static str)> = Vec::new();
    for item in &module.items {
        let (name, span, what) = match item {
            ItemDecl::Fn(f) => (&f.name, f.name_span, "a function"),
            ItemDecl::Struct(s) => {
                if !mente_syntax::is_value_struct(s) {
                    add(&s.name, "a struct", "a `@layout` struct as a value", "M3");
                }
                (&s.name, s.name_span, "a struct")
            }
            ItemDecl::BitStruct(b) => {
                add(&b.name, "a bitstruct", "a bitstruct as a value", "M3");
                (&b.name, b.name_span, "a bitstruct")
            }
            ItemDecl::Enum(e) => {
                add(&e.name, "an enum", "an enum", "M2");
                continue;
            }
            ItemDecl::Trait(t) => {
                add(&t.name, "a trait", "a trait as a type", "M4");
                continue;
            }
            ItemDecl::Unit(_) | ItemDecl::Example(_) => continue,
        };
        if what != "a function" {
            reserved.push((name, span, what));
        }
        items.push((name, span, what));
    }
    let mut errors = clashes(items);
    let mut named_like_the_language = HashSet::new();
    for (name, span, what) in reserved {
        if mente_syntax::body::language_type(name) {
            named_like_the_language.insert(name.to_string());
            errors.push(Diagnostic::error(
                "E0228",
                span,
                format!(
                    "`{}` is a type of the language; a name means one thing, so give this {} another",
                    name,
                    what.trim_start_matches("a ")
                ),
            ));
        }
    }
    let mut structs = Structs {
        defs: Vec::new(),
        later,
    };
    for p in parsed {
        let syntax = p.syntax.clone();
        let fields = syntax
            .fields
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                ty: Ty::of(&syntax.tree, f.ty),
                decl: f.ty,
            })
            .collect();
        structs.defs.push(StructDef {
            name: p.name.clone(),
            syntax,
            fields,
            ok: p.errors.is_empty() && !named_like_the_language.contains(&p.name),
            layout: None,
        });
    }
    let names: HashSet<String> = structs.defs.iter().map(|d| d.name.clone()).collect();
    for i in 0..structs.defs.len() {
        if !structs.defs[i].ok {
            continue;
        }
        let found = check_fields(&structs, &structs.defs[i], &names);
        if !found.is_empty() {
            structs.defs[i].ok = false;
        }
        errors.extend(found);
    }
    let cycles = self_containing(&structs);
    for (i, d) in &cycles {
        structs.defs[*i].ok = false;
        errors.push(d.clone());
    }
    // Layouts in the order of their dependencies: a struct's fields first.
    let mut state = vec![Visit::New; structs.defs.len()];
    for i in 0..structs.defs.len() {
        layout(&mut structs, i, &mut state, &mut errors);
    }
    errors.sort_by_key(|d| d.primary.start);
    (structs, errors)
}

/// `E0228` for each item that takes a name an earlier item of the module declares: each item
/// as its name, its span and what it is, "a function" or "a struct".
pub fn clashes<'a>(
    items: impl IntoIterator<Item = (&'a str, Span, &'static str)>,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut declared: HashMap<&str, (Span, &'static str)> = HashMap::new();
    for (name, span, what) in items {
        match declared.get(name) {
            Some((first, first_what)) => errors.push(
                Diagnostic::error(
                    "E0228",
                    span,
                    format!(
                        "`{}` is already declared in this module, as {}; a name means one thing in a module, so give this {} another",
                        name,
                        first_what,
                        what.trim_start_matches("a ")
                    ),
                )
                .with_related(*first, "first declared here"),
            ),
            None => {
                declared.insert(name, (span, what));
            }
        }
    }
    errors
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Open,
    Done,
}

/// Gives struct `i` its layout, after those of the structs its fields hold, and reports a value
/// too large to keep in one piece (`E0223`): the innermost array type of a field that is, or
/// else the struct.
fn layout(structs: &mut Structs, i: usize, state: &mut [Visit], errors: &mut Vec<Diagnostic>) {
    if state[i] != Visit::New {
        return;
    }
    state[i] = Visit::Open;
    let deps: Vec<usize> = structs.defs[i]
        .fields
        .iter()
        .flat_map(|f| held(&f.ty))
        .filter_map(|n| structs.defs.iter().position(|d| d.name == n))
        .collect();
    for d in deps {
        layout(structs, d, state, errors);
    }
    state[i] = Visit::Done;
    if !structs.defs[i].ok {
        return;
    }
    let def = &structs.defs[i];
    let mut sizes = Vec::new();
    let mut reported = false;
    let mut too_many = false;
    for f in &def.fields {
        let tree = def.tree();
        for (k, node) in tree.types.iter().enumerate() {
            let TypeKind::Array { elem, len } = &node.kind else {
                continue;
            };
            if !within(f.decl, TypeId(k as u32), tree) {
                continue;
            }
            let t = Ty::of(tree, TypeId(k as u32));
            let inner = structs.layout_of(&Ty::of(tree, *elem));
            if let (Some((size, _)), Some((elem_size, _))) = (structs.layout_of(&t), inner) {
                if size > VALUE_BYTES && elem_size <= VALUE_BYTES {
                    reported = true;
                    errors.push(Diagnostic::error(
                        "E0223",
                        node.span,
                        format!(
                            "an array of type `{}` holds {} bytes, more than the {} a value, kept in one piece, may hold",
                            tree.render_type(TypeId(k as u32)),
                            size,
                            VALUE_BYTES
                        ),
                    ));
                } else if *len > ARRAY_ELEMENTS {
                    reported = true;
                    too_many = true;
                    let text = tree.render_type(TypeId(k as u32));
                    errors.push(too_many_elements(node.span, &text, *len));
                }
            }
        }
        match structs.layout_of(&f.ty) {
            Some(l) => sizes.push(l),
            None => return,
        }
    }
    if too_many {
        structs.defs[i].ok = false;
        return;
    }
    let placed = place_fields(&sizes);
    if placed.size > VALUE_BYTES {
        if !reported {
            errors.push(Diagnostic::error(
                "E0223",
                def.syntax.name_span,
                format!(
                    "a struct of type `{}` holds {} bytes, more than the {} a value, kept in one piece, may hold",
                    def.name, placed.size, VALUE_BYTES
                ),
            ));
        }
        structs.defs[i].ok = false;
        return;
    }
    structs.defs[i].layout = Some(placed);
}

/// Whether type node `t` lies within the type expression `root`.
fn within(root: TypeId, t: TypeId, tree: &Tree) -> bool {
    root == t
        || match &tree.ty(root).kind {
            TypeKind::Array { elem, .. } => within(*elem, t, tree),
            _ => false,
        }
}

/// The structs a value of `t` holds in place: `t` itself, or its arrays' elements.
fn held(t: &Ty) -> Vec<String> {
    match t {
        Ty::Struct(name) => vec![name.clone()],
        Ty::Array(_, elem) => held(elem),
        _ => Vec::new(),
    }
}

/// The checks of one declaration's fields: each type is one M2 has, each refinement bound a
/// literal, and each field named once.
fn check_fields(structs: &Structs, def: &StructDef, names: &HashSet<String>) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let tree = def.tree();
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for f in &def.syntax.fields {
        match seen.get(f.name.as_str()) {
            Some(first) => errors.push(
                Diagnostic::error(
                    "E0228",
                    f.name_span,
                    format!(
                        "`{}` is already a field of `{}`; a struct names each field once",
                        f.name, def.name
                    ),
                )
                .with_related(*first, "first declared here"),
            ),
            None => {
                seen.insert(&f.name, f.name_span);
            }
        }
        check_type(structs, def, tree, f.ty, names, &mut errors);
    }
    errors
}

/// The checks of one type in a declaration.
fn check_type(
    structs: &Structs,
    def: &StructDef,
    tree: &Tree,
    t: TypeId,
    names: &HashSet<String>,
    errors: &mut Vec<Diagnostic>,
) {
    let node = tree.ty(t);
    match &node.kind {
        TypeKind::Named(n) => {
            if let Some(d) = unknown_type(structs, n, node.span, names) {
                errors.push(d);
            }
        }
        TypeKind::Array { elem, .. } => check_type(structs, def, tree, *elem, names, errors),
        TypeKind::Int {
            refinement: Some(r),
            ..
        } => {
            for b in [&r.lo, &r.hi] {
                let (Bound::Name(n) | Bound::Len(n)) = b else {
                    continue;
                };
                let d = if def.field(n).is_some() {
                    Diagnostic::error(
                        "E0109",
                        node.span,
                        format!(
                            "a refinement that names `{}`, another field, relates two fields, an invariant, which comes in a later milestone; M1 does not have it",
                            n
                        ),
                    )
                } else {
                    Diagnostic::error(
                        "E0201",
                        node.span,
                        format!(
                            "there is no binding named `{}` here: no binding is visible in a struct's declaration, so a field's refinement has integer literals as its bounds",
                            n
                        ),
                    )
                };
                errors.push(d);
            }
        }
        _ => {}
    }
}

/// The diagnostic for a type named `n` that is not a struct of the module, if it is none: a type
/// whose values a later milestone brings (`E0109`), or a name that is no type at all (`E0116`),
/// with the closest struct's name, or the integer type an odd width resembles, suggested.
pub fn unknown_type(
    structs: &Structs,
    n: &str,
    span: Span,
    names: &HashSet<String>,
) -> Option<Diagnostic> {
    if names.contains(n) {
        return None;
    }
    if let Some(later) = structs.later.get(n) {
        return Some(later.diagnostic(n, span));
    }
    let mut candidates: Vec<String> = names.iter().cloned().collect();
    candidates.sort();
    let close = closest(n, &candidates);
    let odd_width = matches!(n.as_bytes().first(), Some(b'u' | b'i'))
        && n.len() > 1
        && n[1..].bytes().all(|c| c.is_ascii_digit());
    let message = match (&close, odd_width) {
        (Some(c), _) => format!(
            "`{}` is not a type of this module; did you mean `{}`?",
            n, c
        ),
        (None, true) => format!(
            "`{}` is not a type: the integer types are {}, and a field of another width belongs to a bitstruct",
            n,
            INT_TYPES
                .iter()
                .map(|t: &IntTy| format!("`{}`", t.name))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        (None, false) => format!(
            "`{}` is not a type: the types are the integer types, `bool`, arrays `[N]T` and the structs of the module",
            n
        ),
    };
    let mut d = Diagnostic::error("E0116", span, message);
    if let Some(c) = close {
        d = d.with_fix(
            mente_syntax::diagnostics::Applicability::Suggestion,
            format!("write `{}`", c),
            span,
            c,
        );
    }
    Some(d)
}

/// Each struct that contains itself, directly or through other structs and arrays (`E0225`),
/// once for each cycle, at the first of its structs to be declared.
fn self_containing(structs: &Structs) -> Vec<(usize, Diagnostic)> {
    let index: HashMap<&str, usize> = structs
        .defs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name.as_str(), i))
        .collect();
    // Each struct's edges: the field and the struct it holds in place.
    let edges: Vec<Vec<(usize, usize)>> = structs
        .defs
        .iter()
        .map(|d| {
            d.fields
                .iter()
                .enumerate()
                .flat_map(|(k, f)| {
                    held(&f.ty)
                        .into_iter()
                        .filter_map(|n| index.get(n.as_str()).map(|&j| (k, j)))
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .collect();
    let mut found = Vec::new();
    let mut state = vec![Visit::New; structs.defs.len()];
    // The path of the search: each struct with the field it was left by.
    let mut path: Vec<(usize, usize)> = Vec::new();
    for root in 0..structs.defs.len() {
        visit(root, &edges, &mut state, &mut path, &mut found, structs);
    }
    found.sort_by_key(|(i, _)| structs.defs[*i].syntax.name_span.start);
    found
}

/// A depth-first search over the structs, in the order of their declarations and fields. Each
/// field that leads back to a struct still open on the search's path closes a cycle, which is
/// reported once, so that two cycles through one struct are two errors.
fn visit(
    i: usize,
    edges: &[Vec<(usize, usize)>],
    state: &mut [Visit],
    path: &mut Vec<(usize, usize)>,
    found: &mut Vec<(usize, Diagnostic)>,
    structs: &Structs,
) {
    if state[i] != Visit::New {
        return;
    }
    state[i] = Visit::Open;
    for &(field, j) in &edges[i] {
        path.push((i, field));
        match state[j] {
            Visit::Open => {
                let start = path.iter().position(|(s, _)| *s == j).unwrap_or(0);
                let cycle: Vec<(usize, usize)> = path[start..].to_vec();
                // Reported at the struct of the cycle declared first.
                let first = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (s, _))| structs.defs[*s].syntax.name_span.start)
                    .map_or(0, |(k, _)| k);
                let ordered: Vec<(usize, usize)> = cycle[first..]
                    .iter()
                    .chain(&cycle[..first])
                    .copied()
                    .collect();
                found.push((ordered[0].0, cycle_diagnostic(structs, &ordered)));
            }
            Visit::New => visit(j, edges, state, path, found, structs),
            Visit::Done => {}
        }
        path.pop();
    }
    state[i] = Visit::Done;
}

/// `E0225` for a cycle of structs, each with the field that holds the next.
fn cycle_diagnostic(structs: &Structs, cycle: &[(usize, usize)]) -> Diagnostic {
    let def = |s: usize| &structs.defs[s];
    let (first, field) = cycle[0];
    let field_name = &def(first).fields[field].name;
    let message = if cycle.len() == 1 {
        format!(
            "`{}` contains itself through its field `{}`, so a value of it would have no end; a `List<{}>`, which comes later in M2, may hold it, since a list keeps its elements apart",
            def(first).name, field_name, def(first).name
        )
    } else {
        let mut chain = format!("its field `{}` holds `{}`", field_name, {
            let (next, _) = cycle[1];
            &def(next).name
        });
        for k in 1..cycle.len() {
            let (s, f) = cycle[k];
            let next = cycle[(k + 1) % cycle.len()].0;
            chain.push_str(&format!(
                ", whose field `{}` holds `{}`",
                def(s).fields[f].name,
                def(next).name
            ));
        }
        format!(
            "`{}` contains itself: {}, so a value of it would have no end",
            def(first).name,
            chain
        )
    };
    Diagnostic::error("E0225", def(first).syntax.name_span, message).with_related(
        def(first).syntax.fields[field].name_span,
        "this field holds it",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(src: &str) -> (Structs, Vec<Diagnostic>) {
        let module = mente_syntax::parse_module(src).unwrap();
        let parsed = mente_syntax::parse_structs(src, &module);
        declare(&module, &parsed)
    }

    fn codes(src: &str) -> Vec<&'static str> {
        declared(src).1.iter().map(|d| d.code).collect()
    }

    /// Fields are placed by alignment, the largest first, ties in the order of the declaration,
    /// and the size is rounded up to the largest alignment.
    #[test]
    fn test_fields_are_placed_by_alignment_largest_first() {
        // u8, u64, u16, bool, u64: the two u64 first, in their order, then u16, then the bytes.
        let l = place_fields(&[(1, 1), (8, 8), (2, 2), (1, 1), (8, 8)]);
        assert_eq!(l.offsets, vec![18, 0, 16, 19, 8]);
        assert_eq!((l.size, l.align), (24, 8));
        // A u128 is aligned to 16, and the size is rounded up to it.
        let l = place_fields(&[(1, 1), (16, 16)]);
        assert_eq!(l.offsets, vec![16, 0]);
        assert_eq!((l.size, l.align), (32, 16));
        // An array is aligned as its element, and one of no elements takes no bytes.
        let l = place_fields(&[(3, 1), (0, 4), (4, 4)]);
        assert_eq!(l.offsets, vec![4, 0, 0]);
        assert_eq!((l.size, l.align), (8, 4));
        let l = place_fields(&[(1, 1)]);
        assert_eq!((l.offsets.clone(), l.size, l.align), (vec![0], 1, 1));
    }

    #[test]
    fn test_layouts_are_read_from_the_declarations() {
        let (s, errors) = declared(
            "module m\nstruct Inner {\n  a: u8\n  b: u32\n}\nstruct Outer {\n  flag: bool\n  inner: Inner\n  xs: [3]u16\n  wide: i128\n}\n",
        );
        assert!(errors.is_empty(), "{:?}", errors);
        let inner = s.get("Inner").unwrap().layout.clone().unwrap();
        assert_eq!(
            (inner.offsets.clone(), inner.size, inner.align),
            (vec![4, 0], 8, 4)
        );
        let outer = s.get("Outer").unwrap().layout.clone().unwrap();
        // wide (16), inner (8, aligned 4), xs (6, aligned 2), flag (1).
        assert_eq!(outer.offsets, vec![30, 16, 24, 0]);
        assert_eq!((outer.size, outer.align), (32, 16));
        assert_eq!(s.layout_of(&Ty::Struct("Outer".into())), Some((32, 16)));
        assert_eq!(
            s.layout_of(&Ty::Array(2, Box::new(Ty::Struct("Inner".into())))),
            Some((16, 4))
        );
        assert_eq!(s.layout_of(&Ty::Bool), Some((1, 1)));
        assert_eq!(s.layout_of(&Ty::Unit), None);
        assert_eq!(s.layout_of(&Ty::Struct("Nope".into())), None);
    }

    /// A struct that holds itself, directly or through other structs and arrays, is `E0225`,
    /// reported once at the first struct of the cycle; one that holds another is not.
    #[test]
    fn test_a_type_that_contains_itself_is_e0225() {
        assert_eq!(codes("module m\nstruct N {\n  next: N\n}\n"), ["E0225"]);
        let (_, d) = declared(
            "module m\nstruct A {\n  b: [2]B\n}\nstruct B {\n  c: C\n}\nstruct C {\n  a: A\n}\n",
        );
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "`A` contains itself: its field `b` holds `B`, whose field `c` holds `C`, whose field `a` holds `A`, so a value of it would have no end"
        );
        assert_eq!(d[0].primary.line, 2);
        assert!(
            codes("module m\nstruct A {\n  b: B\n  c: [2]B\n}\nstruct B {\n  x: u8\n}\n")
                .is_empty()
        );
        // Two cycles are two errors; a struct that only reaches a cycle is none.
        assert_eq!(
            codes(
                "module m\nstruct A {\n  a: A\n}\nstruct B {\n  b: B\n}\nstruct C {\n  a: A\n}\n"
            ),
            ["E0225", "E0225"]
        );
        // Two cycles through one struct are two errors too, each naming its own.
        let (_, d) = declared(
            "module m\nstruct A {\n  b: B\n  c: C\n}\nstruct B {\n  a: A\n}\nstruct C {\n  a: A\n}\n",
        );
        assert_eq!(
            d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(),
            [
                "`A` contains itself: its field `b` holds `B`, whose field `a` holds `A`, so a value of it would have no end",
                "`A` contains itself: its field `c` holds `C`, whose field `a` holds `A`, so a value of it would have no end",
            ]
        );
        // The structs are searched in the order of their declarations, and each field that
        // leads back into a struct still open on the search closes a cycle, reported once: here
        // A -> B -> A, C -> D -> C, and through B -> C and D -> A, A -> B -> C -> D -> A.
        let (_, d) = declared(
            "module m\nstruct A {\n  b: B\n}\nstruct B {\n  a: A\n  c: C\n}\nstruct C {\n  d: D\n}\nstruct D {\n  c: C\n  a: A\n}\n",
        );
        assert!(d.iter().all(|d| d.code == "E0225"));
        assert_eq!(
            d.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(),
            [
                "`A` contains itself: its field `b` holds `B`, whose field `a` holds `A`, so a value of it would have no end",
                "`A` contains itself: its field `b` holds `B`, whose field `c` holds `C`, whose field `d` holds `D`, whose field `a` holds `A`, so a value of it would have no end",
                "`C` contains itself: its field `d` holds `D`, whose field `c` holds `C`, so a value of it would have no end",
            ]
        );
    }

    #[test]
    fn test_field_types_are_types_m2_has() {
        assert_eq!(
            codes("module m\nstruct S {\n  p: Piar\n}\nstruct Pair {\n  x: u8\n}\n"),
            ["E0116"]
        );
        let (_, d) = declared("module m\nstruct S {\n  p: Piar\n}\nstruct Pair {\n  x: u8\n}\n");
        assert_eq!(
            d[0].message,
            "`Piar` is not a type of this module; did you mean `Pair`?"
        );
        let (_, d) = declared("module m\nstruct S {\n  p: u24\n}\n");
        assert!(
            d[0].message
                .starts_with("`u24` is not a type: the integer types are `u8`"),
            "{}",
            d[0].message
        );
        // A bare `u` is no width at all; an array's element type is checked as a field's.
        let (_, d) = declared("module m\nstruct Segment {\n  p: u\n}\n");
        assert!(
            d[0].message
                .starts_with("`u` is not a type: the types are the integer types"),
            "{}",
            d[0].message
        );
        assert_eq!(
            codes("module m\nstruct S {\n  p: [2][3]Piar\n}\nstruct Pair {\n  x: u8\n}\n"),
            ["E0116"]
        );
        assert_eq!(
            codes("module m\n@layout(size: 1, align: 1)\nstruct L {\n  @bytes(0..1) a: u8\n}\nstruct S {\n  l: L\n}\n"),
            ["E0109"]
        );
        assert_eq!(
            codes("module m\nenum E { A, B }\nstruct S {\n  e: E\n}\n"),
            ["E0109"]
        );
        // A refinement's bound is a literal: another field is an invariant, any other name none.
        assert_eq!(
            codes("module m\nstruct S {\n  lo: u8\n  hi: u8[lo..10]\n}\n"),
            ["E0109"]
        );
        assert_eq!(
            codes("module m\nstruct S {\n  hi: u8[0..n]\n}\n"),
            ["E0201"]
        );
        assert!(codes("module m\nstruct S {\n  hi: i8[-3..=3]\n}\n").is_empty());
    }

    #[test]
    fn test_names_are_declared_once_in_a_module_and_a_struct() {
        assert_eq!(
            codes("module m\nstruct P {\n  i: u8\n  i: u8\n}\n"),
            ["E0228"]
        );
        assert_eq!(
            codes("module m\nstruct P {\n  i: u8\n}\npub fn P() -> u8 = 0\n"),
            ["E0228"]
        );
        assert_eq!(
            codes("module m\npub fn f() -> u8 = 0\npub fn f() -> u8 = 1\n"),
            ["E0228"]
        );
        assert_eq!(
            codes("module m\nstruct P {\n  i: u8\n}\nstruct P {\n  j: u8\n}\n"),
            ["E0228"]
        );
        let (_, d) = declared("module m\npub fn P() -> u8 = 0\nstruct P {\n  i: u8\n}\n");
        assert_eq!(
            d[0].message,
            "`P` is already declared in this module, as a function; a name means one thing in a module, so give this struct another"
        );
        assert_eq!(d[0].related[0].span.line, 2);
        // No struct or bitstruct takes the name of a type of the language: an integer type,
        // `bool`, or one a later milestone brings. Such a struct is not one of the module's.
        for name in [
            "u8", "usize", "i128", "bool", "String", "str", "List", "f64", "Pool",
        ] {
            let (s, d) = declared(&format!(
                "module m
struct {} {{
  a: u8
}}
",
                name
            ));
            assert_eq!(
                d.iter().map(|d| d.code).collect::<Vec<_>>(),
                ["E0228"],
                "{}",
                name
            );
            assert_eq!(
                d[0].message,
                format!(
                    "`{}` is a type of the language; a name means one thing, so give this struct another",
                    name
                )
            );
            assert!(!s.get(name).unwrap().ok, "{}", name);
        }
        assert_eq!(
            codes(
                "module m
@layout(size: 1, align: 1)
struct u16 {
  @bytes(0..1) a: u8
}
"
            ),
            ["E0228"]
        );
        assert_eq!(
            codes(
                "module m
bitstruct Arena : u8 {
  @bits(0..8) raw: u8
}
"
            ),
            ["E0228"]
        );
        // A name the language does not keep is free: `U8`, `Bool`, `u24`.
        for name in ["U8", "Bool", "u24", "Strings"] {
            assert!(
                codes(&format!(
                    "module m
struct {} {{
  a: u8
}}
",
                    name
                ))
                .is_empty(),
                "{}",
                name
            );
        }
    }

    /// A struct holds at most 4,294,967,295 bytes; a field's array beyond them is reported at the
    /// array, and only there.
    #[test]
    fn test_a_struct_holds_at_most_4_gib() {
        assert_eq!(
            codes("module m\nstruct S {\n  a: [4294967295]u8\n  b: u8\n}\n"),
            ["E0223"]
        );
        assert!(codes("module m\nstruct S {\n  a: [4294967294]u8\n  b: u8\n}\n").is_empty());
        let (_, d) = declared("module m\nstruct S {\n  a: [4294967296]u8\n  b: u8\n}\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].primary.line, 3);
        // A struct of structs that together pass the limit.
        assert_eq!(
            codes("module m\nstruct H {\n  a: [3000000000]u8\n}\nstruct S {\n  x: H\n  y: H\n}\n"),
            ["E0223"]
        );
    }

    /// A field's array holds at most 4,294,967,295 elements, whatever their size: elements of
    /// no bytes keep its size within the limit, but not its count.
    #[test]
    fn test_a_fields_array_holds_at_most_4294967295_elements() {
        let z = "module m\nstruct Z {\n  a: [0]u8\n}\n";
        let (s, d) = declared(&format!(
            "{}struct H {{\n  zs: [4294967296]Z\n  b: u8\n}}\n",
            z
        ));
        assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0223"]);
        assert_eq!(
            d[0].message,
            "an array of type `[4294967296]Z` holds 4294967296 elements, more than the 4294967295 an array may hold, whatever their size"
        );
        assert_eq!(d[0].primary.line, 6);
        assert!(!s.get("H").unwrap().ok);
        // Nested, each array is counted: the inner one here.
        assert_eq!(
            codes(&format!("{}struct H {{\n  zs: [2][4294967296]Z\n}}\n", z)),
            ["E0223"]
        );
        let (s, d) = declared(&format!(
            "{}struct H {{\n  zs: [4294967295]Z\n  b: u8\n}}\n",
            z
        ));
        assert!(d.is_empty(), "{:?}", d);
        assert_eq!(s.get("H").unwrap().layout.as_ref().map(|l| l.size), Some(1));
    }

    /// Each array a field's type holds is checked once, at that field, and the innermost too
    /// large is the one reported; a struct is laid out after the structs its fields hold,
    /// wherever they are declared.
    #[test]
    fn test_a_fields_arrays_are_checked_at_that_field() {
        let (_, d) =
            declared("module m\nstruct S {\n  a: u8\n  b: [2][4294967296]u8\n  c: u8\n}\n");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(d[0].code, "E0223");
        // At the inner array `[4294967296]u8`, on the field's line.
        assert_eq!((d[0].primary.line, d[0].primary.col), (4, 9));
        let (s, d) =
            declared("module m\nstruct S {\n  t: T\n  u: [2]T\n}\nstruct T {\n  a: u16\n}\n");
        assert!(d.is_empty(), "{:?}", d);
        assert_eq!(s.get("S").unwrap().layout.as_ref().map(|l| l.size), Some(6));
    }
}

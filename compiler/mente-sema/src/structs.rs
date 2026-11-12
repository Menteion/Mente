//! M2's structs and enums (`spec/core-calculus.md`, items 13, 14 and 21).
//!
//! [`declare`] reads the structs of a module that M2's calculus has, every one without
//! `@layout`, and its enums, checks their declarations, and gives each one that checks its
//! layout. Every later phase reads a struct's fields, an enum's variants and their layouts from
//! the [`Structs`] it returns: the type checker, Tier 1, the reference interpreter and the
//! lowering to MIR, so that each fact about a type is defined once.
//!
//! A declaration's checks: each field's type is one M2 has, a struct or an enum of the module
//! included (`E0116` for a name that is none, `E0109` for a type whose values a later milestone
//! brings); a refinement's bound is a literal, since no binding is visible in a declaration
//! (`E0201`); a name is declared once in a module, none named like a type of the language, a
//! field once in a struct, a variant once in an enum and a field once in a payload (`E0228`); no
//! type contains itself (`E0225`); and a value fits in one piece (`E0223`).
//!
//! Name resolution finds whether a named type is a struct or an enum, and whether `a.b` names a
//! variant: [`qualify`] writes what it finds into a tree, so that every phase after it reads a
//! variant and an enum type as such.

use std::collections::{HashMap, HashSet};

use mente_syntax::diagnostics::Diagnostic;
use mente_syntax::tree::{
    Bound, EnumSyntax, ExprKind, FieldSyntax, Payload, StructSyntax, Tree, TypeId, TypeKind,
};
use mente_syntax::{ItemDecl, ModuleFile, ParsedEnum, ParsedStruct, Span};

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

/// One field of a struct or of a variant's payload: its name, its type, and its declared type
/// in its declaration's tree, which holds its refinement.
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

/// One variant of an enum: its name, and its payload's fields in the order of the declaration,
/// none for a variant that holds nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

impl VariantDef {
    /// The field named `name`, with its index in the payload.
    pub fn field(&self, name: &str) -> Option<(usize, &FieldDef)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

/// Where an enum's tag and payloads lie in its value (`core-calculus.md`, item 21): the tag
/// first, the smallest unsigned integer that numbers its variants, and the payloads after it,
/// sharing their space at the largest payload alignment, the size rounded up to the enum's
/// alignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumLayout {
    pub size: u128,
    pub align: u128,
    /// The tag's width in bytes, at offset 0: 1, 2 or 4.
    pub tag: u128,
    /// Each variant's fields' offsets from the start of the value, in the order of the
    /// declaration.
    pub offsets: Vec<Vec<u128>>,
}

/// The width in bytes of the tag of an enum of `variants` variants: the smallest unsigned
/// integer type that numbers them, `u8` up to 256 of them.
pub fn tag_bytes(variants: usize) -> u128 {
    match variants {
        0..=256 => 1,
        257..=65_536 => 2,
        _ => 4,
    }
}

/// One enum of the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDef {
    pub name: String,
    pub syntax: EnumSyntax,
    pub variants: Vec<VariantDef>,
    /// Whether its declaration parsed and checked without an error.
    pub ok: bool,
    /// Its layout, when its declaration checks.
    pub layout: Option<EnumLayout>,
}

impl EnumDef {
    /// The tree its payloads' declared types live in.
    pub fn tree(&self) -> &Tree {
        &self.syntax.tree
    }

    /// The variant named `name`, with its index in the declaration, which is its tag.
    pub fn variant(&self, name: &str) -> Option<(usize, &VariantDef)> {
        self.variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.name == name)
    }

    /// The names of its variants, in the order of the declaration.
    pub fn variant_names(&self) -> Vec<String> {
        self.variants.iter().map(|v| v.name.clone()).collect()
    }
}

/// The structs and enums of a module, and the names of its other type declarations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Structs {
    pub defs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    /// Each struct's position in `defs` by its name, the first declared of a name: a lookup
    /// does not grow with the number of structs.
    index: HashMap<String, usize>,
    /// Each enum's position in `enums` by its name, likewise.
    enum_index: HashMap<String, usize>,
    /// Each type the module declares whose values M2 does not have.
    pub later: HashMap<String, Later>,
}

/// A type a module declares whose values a later milestone brings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Later {
    /// What the declaration is: "a struct", "a bitstruct" or "a trait".
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
            mente_syntax::diagnostics::later_message(
                &format!("{} (`{}`)", self.what, n),
                Some(self.milestone),
            ),
        )
    }
}

impl Structs {
    /// Adds a struct's declaration, after the module's earlier ones.
    fn push(&mut self, def: StructDef) {
        self.index
            .entry(def.name.clone())
            .or_insert(self.defs.len());
        self.defs.push(def);
    }

    /// Adds an enum's declaration, after the module's earlier ones.
    fn push_enum(&mut self, def: EnumDef) {
        self.enum_index
            .entry(def.name.clone())
            .or_insert(self.enums.len());
        self.enums.push(def);
    }

    /// The position in `defs` of the struct named `name`, the first declared of that name.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn get(&self, name: &str) -> Option<&StructDef> {
        self.position(name).map(|i| &self.defs[i])
    }

    /// Whether the module declares a struct named `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// The position in `enums` of the enum named `name`, the first declared of that name.
    pub fn enum_position(&self, name: &str) -> Option<usize> {
        self.enum_index.get(name).copied()
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enum_position(name).map(|i| &self.enums[i])
    }

    /// Whether the module declares an enum named `name`.
    pub fn is_enum(&self, name: &str) -> bool {
        self.enum_index.contains_key(name)
    }

    /// Whether the module declares a struct or an enum named `name`, a type M2 has.
    pub fn is_type(&self, name: &str) -> bool {
        self.contains(name) || self.is_enum(name)
    }

    /// The names of the module's structs, in the order of their declarations, for suggestions.
    pub fn names(&self) -> Vec<String> {
        self.defs.iter().map(|d| d.name.clone()).collect()
    }

    /// The names of the module's structs and enums, for suggestions.
    pub fn type_names(&self) -> Vec<String> {
        let mut names = self.names();
        names.extend(self.enums.iter().map(|e| e.name.clone()));
        names
    }

    /// The size and alignment in bytes of a value of `t` (`core-calculus.md`, item 21): an
    /// integer takes its width and is aligned to it, `usize` 8 bytes, `bool` 1 byte, `[N]T` `N`
    /// times `T`'s size, aligned as `T`, and a struct or an enum its layout's. An integer of no
    /// one type counts sixteen bytes, the widest machine type lowering may keep it in. `None`
    /// for a type without a layout: no value, any type, or a struct or an enum whose
    /// declaration does not check.
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
            Ty::Enum(name) => self
                .get_enum(name)?
                .layout
                .as_ref()
                .map(|l| (l.size, l.align)),
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

/// Places an enum's tag and payloads (`core-calculus.md`, item 21): the tag of `tag` bytes
/// first, then each payload, its fields placed as a struct's, at the largest of the payloads'
/// alignments, and the size rounded up to the enum's alignment. An enum without payloads is its
/// tag.
pub fn place_variants(tag: u128, payloads: &[Layout]) -> EnumLayout {
    let payload_align = payloads.iter().map(|p| p.align).max().unwrap_or(1).max(1);
    let start = tag.saturating_add(payload_align - 1) / payload_align * payload_align;
    let align = tag.max(payload_align);
    let widest = payloads.iter().map(|p| p.size).max().unwrap_or(0);
    let end = start.saturating_add(widest);
    let size = end.saturating_add(align - 1) / align * align;
    EnumLayout {
        size,
        align,
        tag,
        offsets: payloads
            .iter()
            .map(|p| p.offsets.iter().map(|o| start.saturating_add(*o)).collect())
            .collect(),
    }
}

/// Writes into `tree` what name resolution finds of its names, as `enums` holds the module's
/// enums: a named type that is an enum is one (`TypeKind::Enum`), and `a.b`, `a` an enum, its
/// variant (`ExprKind::Variant`). A tree it has qualified it leaves as it is.
pub fn qualify(tree: &mut Tree, enums: &HashSet<String>) {
    if enums.is_empty() {
        return;
    }
    for t in &mut tree.types {
        if let TypeKind::Named(n) = &t.kind {
            if enums.contains(n) {
                t.kind = TypeKind::Enum(n.clone());
            }
        }
    }
    for i in 0..tree.exprs.len() {
        let ExprKind::Field { base, name } = &tree.exprs[i].kind else {
            continue;
        };
        let base = tree.exprs[base.0 as usize].clone();
        let ExprKind::Name(n) = &base.kind else {
            continue;
        };
        if !enums.contains(n) {
            continue;
        }
        let span = tree.exprs[i].span;
        let name = name.clone();
        let name_span = Span {
            start: span.end - name.len(),
            col: span.col + (span.end - span.start - name.len()) as u32,
            ..span
        };
        tree.exprs[i].kind = ExprKind::Variant {
            enum_name: n.clone(),
            enum_span: base.span,
            name,
            name_span,
            payload: Payload::None,
        };
    }
}

/// The names of the module's enums, which [`qualify`] reads.
pub fn enum_names(module: &ModuleFile) -> HashSet<String> {
    module
        .items
        .iter()
        .filter_map(|i| match i {
            ItemDecl::Enum(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect()
}

/// Reads and checks the structs and enums of `module` that `parsed` and `parsed_enums` hold,
/// and the names its items declare. The diagnostics come in the order of the module.
pub fn declare(
    module: &ModuleFile,
    parsed: &[ParsedStruct],
    parsed_enums: &[ParsedEnum],
) -> (Structs, Vec<Diagnostic>) {
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
    // A name is declared once in a module: a function, a struct of any kind, a bitstruct or an
    // enum.
    let mut items: Vec<(&str, Span, &'static str)> = Vec::new();
    // And no struct, bitstruct or enum takes the name of a type of the language.
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
            ItemDecl::Enum(e) => (&e.name, e.name_span, "an enum"),
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
                    what.trim_start_matches("a ").trim_start_matches("an ")
                ),
            ));
        }
    }
    let enums = enum_names(module);
    let mut structs = Structs {
        later,
        ..Structs::default()
    };
    let fields_of = |syntax: &[FieldSyntax], tree: &Tree| -> Vec<FieldDef> {
        syntax
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                ty: Ty::of(tree, f.ty),
                decl: f.ty,
            })
            .collect()
    };
    for p in parsed {
        let mut syntax = p.syntax.clone();
        qualify(&mut syntax.tree, &enums);
        let fields = fields_of(&syntax.fields, &syntax.tree);
        structs.push(StructDef {
            name: p.name.clone(),
            syntax,
            fields,
            ok: p.errors.is_empty() && !named_like_the_language.contains(&p.name),
            layout: None,
        });
    }
    for p in parsed_enums {
        let mut syntax = p.syntax.clone();
        qualify(&mut syntax.tree, &enums);
        let variants = syntax
            .variants
            .iter()
            .map(|v| VariantDef {
                name: v.name.clone(),
                fields: fields_of(&v.fields, &syntax.tree),
            })
            .collect();
        structs.push_enum(EnumDef {
            name: p.name.clone(),
            syntax,
            variants,
            ok: p.errors.is_empty() && !named_like_the_language.contains(&p.name),
            layout: None,
        });
    }
    for i in 0..structs.defs.len() {
        if !structs.defs[i].ok {
            continue;
        }
        let found = check_fields(&structs, &structs.defs[i]);
        if !found.is_empty() {
            structs.defs[i].ok = false;
        }
        errors.extend(found);
    }
    for i in 0..structs.enums.len() {
        if !structs.enums[i].ok {
            continue;
        }
        let found = check_variants(&structs, &structs.enums[i]);
        if !found.is_empty() {
            structs.enums[i].ok = false;
        }
        errors.extend(found);
    }
    let graph = Graph::of(&structs);
    for (node, d) in self_containing(&structs, &graph) {
        match node {
            Node::Struct(i) => structs.defs[i].ok = false,
            Node::Enum(i) => structs.enums[i].ok = false,
        }
        errors.push(d);
    }
    // Layouts in the order of their dependencies: the types a value holds first.
    let mut state = vec![Visit::New; graph.nodes.len()];
    for k in 0..graph.nodes.len() {
        layout(&mut structs, &graph, k, &mut state, &mut errors);
    }
    errors.sort_by_key(|d| d.primary.start);
    (structs, errors)
}

/// `E0228` for each item that takes a name an earlier item of the module declares: each item
/// as its name, its span and what it is, "a function", "a struct" or "an enum".
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
                        what.trim_start_matches("a ").trim_start_matches("an ")
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

/// A struct or an enum of the module, by its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Struct(usize),
    Enum(usize),
}

/// Where one type holds another in place: a struct's field, or a field of an enum's variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Via {
    variant: Option<usize>,
    field: usize,
}

/// The structs and enums of a module as nodes, structs first, each with the types its fields
/// hold in place, by node: an edge for each, in the order of its fields and variants.
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Vec<(Via, usize)>>,
}

impl Graph {
    fn of(structs: &Structs) -> Graph {
        let s = structs.defs.len();
        let mut nodes: Vec<Node> = (0..s).map(Node::Struct).collect();
        nodes.extend((0..structs.enums.len()).map(Node::Enum));
        let node_of = |name: &str| {
            structs
                .position(name)
                .or_else(|| structs.enum_position(name).map(|e| s + e))
        };
        let targets = |fields: &[FieldDef], variant: Option<usize>| -> Vec<(Via, usize)> {
            fields
                .iter()
                .enumerate()
                .flat_map(|(k, f)| {
                    held(&f.ty)
                        .into_iter()
                        .filter_map(|n| node_of(&n))
                        .map(|j| (Via { variant, field: k }, j))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let edges = nodes
            .iter()
            .map(|n| match *n {
                Node::Struct(i) => targets(&structs.defs[i].fields, None),
                Node::Enum(i) => structs.enums[i]
                    .variants
                    .iter()
                    .enumerate()
                    .flat_map(|(v, var)| targets(&var.fields, Some(v)))
                    .collect(),
            })
            .collect();
        Graph { nodes, edges }
    }
}

/// The name of a node and where it is declared.
fn node_name(structs: &Structs, n: Node) -> (&str, Span) {
    match n {
        Node::Struct(i) => (&structs.defs[i].name, structs.defs[i].syntax.name_span),
        Node::Enum(i) => (&structs.enums[i].name, structs.enums[i].syntax.name_span),
    }
}

/// The name of the field `via` names in node `n`, and where it is declared.
fn via_field(structs: &Structs, n: Node, via: Via) -> (&str, Span) {
    match (n, via.variant) {
        (Node::Struct(i), _) => {
            let f = &structs.defs[i].syntax.fields[via.field];
            (&f.name, f.name_span)
        }
        (Node::Enum(i), Some(v)) => {
            let f = &structs.enums[i].syntax.variants[v].fields[via.field];
            (&f.name, f.name_span)
        }
        (Node::Enum(i), None) => (&structs.enums[i].name, structs.enums[i].syntax.name_span),
    }
}

/// The name of the variant `via` names in node `n`, when it is an enum's.
fn via_variant(structs: &Structs, n: Node, via: Via) -> Option<&str> {
    match (n, via.variant) {
        (Node::Enum(i), Some(v)) => Some(&structs.enums[i].variants[v].name),
        _ => None,
    }
}

/// Gives node `k` its layout, after those of the types its fields hold, and reports a value too
/// large to keep in one piece (`E0223`): the innermost array type of a field that is, or else
/// the struct or the enum.
fn layout(
    structs: &mut Structs,
    graph: &Graph,
    k: usize,
    state: &mut [Visit],
    errors: &mut Vec<Diagnostic>,
) {
    if state[k] != Visit::New {
        return;
    }
    state[k] = Visit::Open;
    for &(_, d) in &graph.edges[k] {
        layout(structs, graph, d, state, errors);
    }
    state[k] = Visit::Done;
    match graph.nodes[k] {
        Node::Struct(i) => struct_layout(structs, i, errors),
        Node::Enum(i) => enum_layout(structs, i, errors),
    }
}

/// The sizes and alignments of `fields`, declared in `tree`, after reporting each array type
/// among them too large to keep in one piece (`E0223`); `None` when a field has no layout, and
/// whether one was reported, and whether one held too many elements.
fn field_sizes(
    structs: &Structs,
    tree: &Tree,
    fields: &[FieldDef],
    errors: &mut Vec<Diagnostic>,
) -> (Option<Vec<(u128, u128)>>, bool, bool) {
    let mut sizes = Vec::new();
    let mut reported = false;
    let mut too_many = false;
    for f in fields {
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
            None => return (None, reported, too_many),
        }
    }
    (Some(sizes), reported, too_many)
}

fn struct_layout(structs: &mut Structs, i: usize, errors: &mut Vec<Diagnostic>) {
    if !structs.defs[i].ok {
        return;
    }
    let def = &structs.defs[i];
    let (sizes, reported, too_many) = field_sizes(structs, def.tree(), &def.fields, errors);
    let Some(sizes) = sizes else {
        return;
    };
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

fn enum_layout(structs: &mut Structs, i: usize, errors: &mut Vec<Diagnostic>) {
    if !structs.enums[i].ok {
        return;
    }
    let def = &structs.enums[i];
    let mut payloads = Vec::new();
    let mut reported = false;
    let mut too_many = false;
    for v in &def.variants {
        let (sizes, r, t) = field_sizes(structs, def.tree(), &v.fields, errors);
        reported |= r;
        too_many |= t;
        match sizes {
            Some(sizes) => payloads.push(place_fields(&sizes)),
            None => return,
        }
    }
    if too_many {
        structs.enums[i].ok = false;
        return;
    }
    let placed = place_variants(tag_bytes(def.variants.len()), &payloads);
    if placed.size > VALUE_BYTES {
        if !reported {
            errors.push(Diagnostic::error(
                "E0223",
                def.syntax.name_span,
                format!(
                    "an enum of type `{}` holds {} bytes, its tag and its largest payload, more than the {} a value, kept in one piece, may hold",
                    def.name, placed.size, VALUE_BYTES
                ),
            ));
        }
        structs.enums[i].ok = false;
        return;
    }
    structs.enums[i].layout = Some(placed);
}

/// Whether type node `t` lies within the type expression `root`.
fn within(root: TypeId, t: TypeId, tree: &Tree) -> bool {
    root == t
        || match &tree.ty(root).kind {
            TypeKind::Array { elem, .. } => within(*elem, t, tree),
            _ => false,
        }
}

/// The structs and enums a value of `t` holds in place: `t` itself, or its arrays' elements.
fn held(t: &Ty) -> Vec<String> {
    match t {
        Ty::Struct(name) | Ty::Enum(name) => vec![name.clone()],
        Ty::Array(_, elem) => held(elem),
        _ => Vec::new(),
    }
}

/// The checks of one declaration's fields: each type is one M2 has, each refinement bound a
/// literal, and each field named once.
fn check_fields(structs: &Structs, def: &StructDef) -> Vec<Diagnostic> {
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
        check_type(
            structs,
            &|n| def.field(n).is_some(),
            tree,
            f.ty,
            &mut errors,
        );
    }
    errors
}

/// The checks of one enum's variants: each named once, each payload naming each field once, and
/// each payload field's type checked as a struct field's.
fn check_variants(structs: &Structs, def: &EnumDef) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let tree = def.tree();
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for v in &def.syntax.variants {
        match seen.get(v.name.as_str()) {
            Some(first) => errors.push(
                Diagnostic::error(
                    "E0228",
                    v.name_span,
                    format!(
                        "`{}` is already a variant of `{}`; an enum names each variant once",
                        v.name, def.name
                    ),
                )
                .with_related(*first, "first declared here"),
            ),
            None => {
                seen.insert(&v.name, v.name_span);
            }
        }
        let mut fields: HashMap<&str, Span> = HashMap::new();
        for f in &v.fields {
            match fields.get(f.name.as_str()) {
                Some(first) => errors.push(
                    Diagnostic::error(
                        "E0228",
                        f.name_span,
                        format!(
                            "`{}` is already a field of `{}.{}`; a payload names each field once",
                            f.name, def.name, v.name
                        ),
                    )
                    .with_related(*first, "first declared here"),
                ),
                None => {
                    fields.insert(&f.name, f.name_span);
                }
            }
            let is_field = |n: &str| v.fields.iter().any(|g| g.name == n);
            check_type(structs, &is_field, tree, f.ty, &mut errors);
        }
    }
    errors
}

/// The checks of one type in a declaration; `is_field` tells a name of another field of the
/// same struct or payload.
fn check_type(
    structs: &Structs,
    is_field: &dyn Fn(&str) -> bool,
    tree: &Tree,
    t: TypeId,
    errors: &mut Vec<Diagnostic>,
) {
    let node = tree.ty(t);
    match &node.kind {
        TypeKind::Named(n) | TypeKind::Enum(n) => {
            if let Some(d) = unknown_type(structs, n, node.span) {
                errors.push(d);
            }
        }
        TypeKind::Array { elem, .. } => check_type(structs, is_field, tree, *elem, errors),
        TypeKind::Int {
            refinement: Some(r),
            ..
        } => {
            for b in [&r.lo, &r.hi] {
                let (Bound::Name(n) | Bound::Len(n)) = b else {
                    continue;
                };
                let d = if is_field(n) {
                    Diagnostic::error(
                        "E0109",
                        node.span,
                        mente_syntax::diagnostics::later_message(
                            &format!(
                                "a refinement that names `{}`, another field, relates two fields, an invariant, which",
                                n
                            ),
                            Some("a later milestone"),
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
        TypeKind::Int {
            refinement: None, ..
        }
        | TypeKind::Bool
        | TypeKind::Error => {}
    }
}

/// The diagnostic for a type named `n` that is not a struct or an enum of the module, if it is
/// none: a type whose values a later milestone brings (`E0109`), or a name that is no type at
/// all (`E0116`), with the closest type's name, or the integer type an odd width resembles,
/// suggested.
pub fn unknown_type(structs: &Structs, n: &str, span: Span) -> Option<Diagnostic> {
    if structs.is_type(n) {
        return None;
    }
    if let Some(later) = structs.later.get(n) {
        return Some(later.diagnostic(n, span));
    }
    let mut candidates = structs.type_names();
    candidates.sort();
    candidates.dedup();
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
            "`{}` is not a type: the types are the integer types, `bool`, arrays `[N]T` and the structs and enums of the module",
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

/// Each struct or enum that contains itself, directly or through other structs, enums and
/// arrays (`E0225`), once for each cycle, at the first of its types to be declared.
fn self_containing(structs: &Structs, graph: &Graph) -> Vec<(Node, Diagnostic)> {
    let mut found = Vec::new();
    let mut state = vec![Visit::New; graph.nodes.len()];
    // The path of the search: each node with the edge it was left by.
    let mut path: Vec<(usize, Via)> = Vec::new();
    for root in 0..graph.nodes.len() {
        visit(root, graph, &mut state, &mut path, &mut found, structs);
    }
    found.sort_by_key(|(n, _)| node_name(structs, *n).1.start);
    found
}

/// A depth-first search over the types, in the order of their declarations and fields. Each
/// field that leads back to a type still open on the search's path closes a cycle, which is
/// reported once, so that two cycles through one type are two errors.
fn visit(
    i: usize,
    graph: &Graph,
    state: &mut [Visit],
    path: &mut Vec<(usize, Via)>,
    found: &mut Vec<(Node, Diagnostic)>,
    structs: &Structs,
) {
    if state[i] != Visit::New {
        return;
    }
    state[i] = Visit::Open;
    for &(via, j) in &graph.edges[i] {
        path.push((i, via));
        match state[j] {
            Visit::Open => {
                let start = path.iter().position(|(s, _)| *s == j).unwrap_or(0);
                let cycle: Vec<(usize, Via)> = path[start..].to_vec();
                // Reported at the type of the cycle declared first.
                let first = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (s, _))| node_name(structs, graph.nodes[*s]).1.start)
                    .map_or(0, |(k, _)| k);
                let ordered: Vec<(Node, Via)> = cycle[first..]
                    .iter()
                    .chain(&cycle[..first])
                    .map(|(s, v)| (graph.nodes[*s], *v))
                    .collect();
                found.push((ordered[0].0, cycle_diagnostic(structs, &ordered)));
            }
            Visit::New => visit(j, graph, state, path, found, structs),
            Visit::Done => {}
        }
        path.pop();
    }
    state[i] = Visit::Done;
}

/// `E0225` for a cycle of types, each with the field that holds the next.
fn cycle_diagnostic(structs: &Structs, cycle: &[(Node, Via)]) -> Diagnostic {
    let (first, via) = cycle[0];
    let (first_name, first_span) = node_name(structs, first);
    let (_, field_span) = via_field(structs, first, via);
    // How a type holds the next: "its field `f`", or "the field `f` of its variant `V`".
    let through = |n: Node, v: Via| {
        let (f, _) = via_field(structs, n, v);
        match via_variant(structs, n, v) {
            Some(var) => format!("the field `{}` of its variant `{}`", f, var),
            None => format!("its field `{}`", f),
        }
    };
    let message = if cycle.len() == 1 {
        format!(
            "`{}` contains itself through {}, so a value of it would have no end; a `List<{}>`, which comes later in M2, may hold it, since a list keeps its elements apart",
            first_name,
            through(first, via),
            first_name
        )
    } else {
        let next_name = |k: usize| node_name(structs, cycle[(k + 1) % cycle.len()].0).0;
        let mut chain = format!("{} holds `{}`", through(first, via), next_name(0));
        for (k, &(n, v)) in cycle.iter().enumerate().skip(1) {
            let (f, _) = via_field(structs, n, v);
            let step = match via_variant(structs, n, v) {
                Some(var) => format!(
                    ", whose variant `{}` holds `{}` in its field `{}`",
                    var,
                    next_name(k),
                    f
                ),
                None => format!(", whose field `{}` holds `{}`", f, next_name(k)),
            };
            chain.push_str(&step);
        }
        format!(
            "`{}` contains itself: {}, so a value of it would have no end",
            first_name, chain
        )
    };
    Diagnostic::error("E0225", first_span, message).with_related(field_span, "this field holds it")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(src: &str) -> (Structs, Vec<Diagnostic>) {
        let module = mente_syntax::parse_module(src).unwrap();
        let parsed = mente_syntax::parse_structs(src, &module);
        let enums = mente_syntax::parse_enums(src, &module);
        declare(&module, &parsed, &enums)
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
        // An enum of the module is a type M2 has.
        assert!(codes("module m\nenum E {\n  A\n  B\n}\nstruct S {\n  e: E\n}\n").is_empty());
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

    /// A struct is found by its name through an index, the first declared of a name, so that a
    /// lookup does not grow with the number of structs; a long chain of structs, each holding
    /// the one before, is laid out whole.
    #[test]
    fn test_a_struct_is_found_by_its_name_through_an_index() {
        let (s, _) = declared(
            "module m\nstruct A {\n  x: u8\n}\nstruct B {\n  a: A\n}\nstruct A {\n  y: u16\n}\n",
        );
        assert_eq!(s.index.len(), 2);
        for (name, &i) in &s.index {
            assert_eq!(&s.defs[i].name, name);
        }
        assert_eq!(s.position("A"), Some(0));
        assert_eq!(s.position("B"), Some(1));
        assert_eq!(s.get("A").map(|d| d.fields[0].name.as_str()), Some("x"));
        assert!(s.contains("B") && !s.contains("C"));
        assert_eq!(s.position("C"), None);
        let n = 3000;
        let mut src = String::from("module m\nstruct S0 {\n  a: u8\n}\n");
        for i in 1..n {
            src.push_str(&format!("struct S{} {{\n  p: S{}\n  a: u8\n}}\n", i, i - 1));
        }
        let (s, d) = declared(&src);
        assert!(d.is_empty(), "{:?}", &d[..d.len().min(3)]);
        assert_eq!(s.index.len(), n);
        let last = format!("S{}", n - 1);
        assert_eq!(
            s.get(&last).and_then(|d| d.layout.as_ref()).map(|l| l.size),
            Some(n as u128)
        );
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

    /// The variants of `n` plain variants, one per line.
    fn plain_enum(n: usize) -> String {
        let mut src = String::from("module m\nenum E {\n");
        for k in 0..n {
            src.push_str(&format!("  V{}\n", k));
        }
        src.push_str("}\n");
        src
    }

    /// An enum's tag is the smallest unsigned integer that numbers its variants, first; the
    /// payloads share the space after it at the largest payload alignment, each placed as a
    /// struct's fields are; the size is rounded up to the enum's alignment, and an enum without
    /// payloads is its tag.
    #[test]
    fn test_enums_are_laid_out_tag_first() {
        assert_eq!(tag_bytes(1), 1);
        assert_eq!(tag_bytes(256), 1);
        assert_eq!(tag_bytes(257), 2);
        assert_eq!(tag_bytes(65_536), 2);
        assert_eq!(tag_bytes(65_537), 4);
        for (n, size) in [(1, 1), (256, 1), (257, 2)] {
            let (s, d) = declared(&plain_enum(n));
            assert!(d.is_empty(), "{:?}", d);
            let l = s.get_enum("E").unwrap().layout.clone().unwrap();
            assert_eq!(
                (l.size, l.align, l.tag),
                (size, size, size),
                "{} variants",
                n
            );
            assert_eq!(s.layout_of(&Ty::Enum("E".into())), Some((size, size)));
        }
        let (s, d) = declared(
            "module m\nstruct P {\n  a: u8\n  b: u32\n}\nenum Shape {\n  Dot\n  Rect(w: u16, h: u8)\n  Wide(x: u128, flag: bool)\n  Point(p: P)\n}\n",
        );
        assert!(d.is_empty(), "{:?}", d);
        let l = s.get_enum("Shape").unwrap().layout.clone().unwrap();
        // The payloads start at 16, `Wide`'s alignment; `Wide` is the largest, 17 bytes.
        assert_eq!((l.tag, l.align, l.size), (1, 16, 48));
        assert_eq!(
            l.offsets,
            vec![vec![], vec![16, 18], vec![16, 32], vec![16]]
        );
        // A payload of bytes after a tag of two, and an enum of enums.
        let mut src = plain_enum(300);
        src.push_str("enum F {\n  A(e: E, k: u8)\n  B\n}\nstruct H {\n  f: F\n  n: u16\n}\n");
        let (s, d) = declared(&src);
        assert!(d.is_empty(), "{:?}", d);
        let f = s.get_enum("F").unwrap().layout.clone().unwrap();
        assert_eq!((f.tag, f.align, f.size), (1, 2, 6));
        assert_eq!(f.offsets[0], vec![2, 4]);
        let h = s.get("H").unwrap().layout.clone().unwrap();
        assert_eq!((h.size, h.offsets.clone()), (8, vec![0, 6]));
        // The layout is the one place each fact is defined: `place_variants` alone.
        let p = place_variants(
            2,
            &[place_fields(&[(1, 1)]), place_fields(&[(8, 8), (1, 1)])],
        );
        assert_eq!(
            (p.size, p.align, p.offsets.clone()),
            (24, 8, vec![vec![8], vec![8, 16]])
        );
        assert_eq!(place_variants(1, &[]).size, 1);
    }

    /// An enum declares each variant once and each payload field once in a variant; it takes no
    /// name of the language's; its payload's types are M2's; and it contains itself through no
    /// field, as a struct does not, each cycle named through the variants it passes.
    #[test]
    fn test_enum_declarations_check_their_names_and_types() {
        let d = |src: &str| declared(src).1;
        assert_eq!(codes("module m\nenum E {\n  A\n  B\n  A\n}\n"), ["E0228"]);
        assert_eq!(
            d("module m\nenum E {\n  A(x: u8, x: u16)\n}\n")[0].message,
            "`x` is already a field of `E.A`; a payload names each field once"
        );
        assert_eq!(
            codes("module m\nenum E {\n  A(x: u8)\n  B(x: u16)\n}\n"),
            NONE
        );
        assert_eq!(codes("module m\nenum bool {\n  A\n}\n"), ["E0228"]);
        assert_eq!(
            codes("module m\nenum E {\n  A\n}\nstruct E {\n  x: u8\n}\n"),
            ["E0228"]
        );
        assert_eq!(
            codes("module m\nenum E {\n  A(p: Piar)\n}\nstruct Pair {\n  x: u8\n}\n"),
            ["E0116"]
        );
        // A type of a later milestone is the parser's to report; such an enum is not laid out.
        let (s, _) = declared("module m\nenum E {\n  A(s: String)\n}\n");
        assert!(!s.get_enum("E").unwrap().ok && s.get_enum("E").unwrap().layout.is_none());
        assert_eq!(
            codes("module m\nenum E {\n  A(n: u8[0..m])\n}\n"),
            ["E0201"]
        );
        assert_eq!(
            codes("module m\nenum E {\n  A(lo: u8, hi: u8[lo..9])\n}\n"),
            ["E0109"]
        );
        let (_, d) = declared("module m\nenum L {\n  End\n  Cons(head: u8, tail: L)\n}\n");
        assert_eq!(
            d[0].message,
            "`L` contains itself through the field `tail` of its variant `Cons`, so a value of it would have no end; a `List<L>`, which comes later in M2, may hold it, since a list keeps its elements apart"
        );
        let (s, d) = declared(
            "module m\nstruct N {\n  v: u8\n  next: [2]M\n}\nenum M {\n  Done\n  More(n: N)\n}\n",
        );
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "`N` contains itself: its field `next` holds `M`, whose variant `More` holds `N` in its field `n`, so a value of it would have no end"
        );
        assert!(!s.get("N").unwrap().ok && s.get_enum("M").unwrap().layout.is_none());
        // An enum too large to be a value, at the enum or at its payload's array.
        assert_eq!(
            codes("module m\nenum E {\n  A\n  B(xs: [4294967295]u8)\n}\n"),
            ["E0223"]
        );
        assert_eq!(
            codes("module m\nenum E {\n  A\n  B(xs: [4294967296]u8)\n}\n"),
            ["E0223"]
        );
        assert_eq!(
            codes("module m\nenum E {\n  A\n  B(xs: [4294967288]u8)\n}\n"),
            NONE
        );
        // A tree names an enum as such once qualified.
        let module = mente_syntax::parse_module("module m\nenum E {\n  A\n}\n").unwrap();
        let names = enum_names(&module);
        let src = "module m\nenum E {\n  A\n}\npub fn f(e: E) -> E = E.A\n";
        let m = mente_syntax::parse_module(src).unwrap();
        let mut f = mente_syntax::parse_bodies(src, &m)
            .remove(0)
            .syntax
            .unwrap();
        qualify(&mut f.tree, &names);
        assert_eq!(
            mente_syntax::tree::Tree::render_fn(&f),
            "(fn f ((e E)) E () (= E.A))"
        );
        assert!(matches!(f.tree.ty(f.params[0].ty).kind, TypeKind::Enum(_)));
    }

    const NONE: [&str; 0] = [];
}

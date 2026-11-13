//! Lowering a checked module into MIR (`spec/mir-and-codegen.md` item 2).
//!
//! Each function with a body becomes a MIR function, and each also gets an entry, which takes
//! arguments from outside the program, each already narrowed into its parameter's type by
//! whoever calls the entry ([`crate::native::refusal`]), checks the rest as a call does, and
//! then calls it. The lowering follows
//! the reference interpreter (`compiler/mente-interp`) step for step, so that the MIR
//! interpreter and native code trap where it does and say what it says:
//!
//! - integer arithmetic is exact, each operation in a machine type wide enough for its static
//!   range (`core-calculus.md` item 4), and an operation with an `i128` or `u128` operand is
//!   checked at the operator;
//! - a value is narrowed, and checked against its refinement, where it lands;
//! - at a call, the caller narrows the arguments at their own positions, checks the
//!   parameters' refinements, the callee's `requires` at the call, and its `decreases` measure,
//!   because those traps are located in the caller; the callee checks its `ensures` at each
//!   return;
//! - a `var`, every array and every struct live in memory, a struct's fields where the layout of
//!   `mente_sema::structs` places them, and a `mut` argument is the address of its place.
//!
//! Every check is kept: erasing those Tier 1 proves is a separate step.

use std::collections::HashMap;
use std::sync::Arc;

use mente_mir::cfg::{
    self, ArithOp, BitOp, BlockId, Builder, CmpOp, FuncId, IntTy, Loc, Num, Op, Term, Ty,
};
use mente_mir::NodeId;
use mente_sema::names::BindingKind;
use mente_sema::structs::Structs;
use mente_sema::tier1::{Obligation, ObligationKind, Tag};
use mente_sema::types::{Int as SemaInt, IntTy as SemaIntTy, Ty as SemaTy};
use mente_sema::wide::{self, Interval};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    Arm, ArmBody, AssignOp, BinaryOp, BlockId as AstBlock, Body, Bound, ContractKind, Else, ExprId,
    ExprKind, PatternKind, Payload, Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

/// A lowered module: its MIR, and for each function with a body its entry.
#[derive(Clone, Debug)]
pub struct Lowered {
    pub module: cfg::Module,
    pub entries: Vec<Entry>,
}

/// What calling a function from outside the program takes: its entry, and the shapes of its
/// parameters, each with whether it is `mut`, and of its result.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub func: FuncId,
    pub params: Vec<(bool, Shape)>,
    pub result: Shape,
    /// The function's name in the source, where a call from outside the program reports an
    /// argument its parameter's type cannot hold.
    pub at: Loc,
}

/// The shape of a value: an integer of a type, a boolean, an array, a struct, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Int(SemaIntTy),
    Bool,
    Array(u32, Box<Shape>),
    /// A struct, its fields where its layout places them.
    Struct(Arc<StructShape>),
    /// An enum, its tag first and each variant's payload where its layout places it.
    Enum(Arc<EnumShape>),
    Unit,
}

/// A struct as memory holds it: its fields in the order of its declaration, each at the offset
/// its layout gives it, and its size and alignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructShape {
    pub name: String,
    pub fields: Vec<FieldShape>,
    pub size: u32,
    pub align: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldShape {
    pub name: String,
    pub offset: u32,
    pub shape: Shape,
}

/// An enum as memory holds it: its tag at offset 0, of the smallest unsigned type that numbers
/// its variants, and each variant's payload fields at the offsets its layout gives them, from the
/// start of the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumShape {
    pub name: String,
    pub tag: SemaIntTy,
    pub variants: Vec<VariantShape>,
    pub size: u32,
    pub align: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantShape {
    pub name: String,
    pub fields: Vec<FieldShape>,
}

impl EnumShape {
    /// The variant named `name`, with its index in the declaration, which is its tag.
    pub fn variant(&self, name: &str) -> Option<(usize, &VariantShape)> {
        self.variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.name == name)
    }
}

impl VariantShape {
    /// The field named `name`, with its index in the payload.
    pub fn field(&self, name: &str) -> Option<(usize, &FieldShape)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

impl StructShape {
    /// The field named `name`, with its index in the declaration.
    pub fn field(&self, name: &str) -> Option<(usize, &FieldShape)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

/// The shape of each struct and enum of a module, made from the layouts the checker gives them
/// (`mente_sema::structs`), which is the one place a layout is defined.
#[derive(Clone, Debug, Default)]
pub struct Shapes(
    HashMap<String, Arc<StructShape>>,
    HashMap<String, Arc<EnumShape>>,
);

impl Shapes {
    pub fn of(structs: &Structs) -> Result<Shapes, String> {
        let mut shapes = Shapes::default();
        for d in &structs.defs {
            shapes.add(structs, &d.name)?;
        }
        for e in &structs.enums {
            shapes.add_enum(structs, &e.name)?;
        }
        Ok(shapes)
    }

    fn add(&mut self, structs: &Structs, name: &str) -> Result<Arc<StructShape>, String> {
        if let Some(s) = self.0.get(name) {
            return Ok(s.clone());
        }
        let def = structs
            .get(name)
            .ok_or_else(|| format!("no struct `{}`", name))?;
        let layout = def
            .layout
            .as_ref()
            .ok_or_else(|| format!("a struct `{}` without a layout", name))?;
        let bytes = |n: u128| u32::try_from(n).map_err(|_| format!("a struct of {} bytes", n));
        let mut fields = Vec::new();
        for (f, &offset) in def.fields.iter().zip(&layout.offsets) {
            let shape = self.shape_of(structs, &f.ty)?;
            fields.push(FieldShape {
                name: f.name.clone(),
                offset: bytes(offset)?,
                shape,
            });
        }
        let s = Arc::new(StructShape {
            name: name.to_string(),
            fields,
            size: bytes(layout.size)?,
            align: bytes(layout.align)?,
        });
        self.0.insert(name.to_string(), s.clone());
        Ok(s)
    }

    fn add_enum(&mut self, structs: &Structs, name: &str) -> Result<Arc<EnumShape>, String> {
        if let Some(e) = self.1.get(name) {
            return Ok(e.clone());
        }
        let def = structs
            .get_enum(name)
            .ok_or_else(|| format!("no enum `{}`", name))?;
        let layout = def
            .layout
            .as_ref()
            .ok_or_else(|| format!("an enum `{}` without a layout", name))?;
        let bytes = |n: u128| u32::try_from(n).map_err(|_| format!("an enum of {} bytes", n));
        let tag = SemaIntTy::named(match layout.tag {
            1 => "u8",
            2 => "u16",
            _ => "u32",
        })
        .ok_or("a tag of no integer type")?;
        let mut variants = Vec::new();
        for (v, offsets) in def.variants.iter().zip(&layout.offsets) {
            let mut fields = Vec::new();
            for (f, &offset) in v.fields.iter().zip(offsets) {
                let shape = self.shape_of(structs, &f.ty)?;
                fields.push(FieldShape {
                    name: f.name.clone(),
                    offset: bytes(offset)?,
                    shape,
                });
            }
            variants.push(VariantShape {
                name: v.name.clone(),
                fields,
            });
        }
        let e = Arc::new(EnumShape {
            name: name.to_string(),
            tag,
            variants,
            size: bytes(layout.size)?,
            align: bytes(layout.align)?,
        });
        self.1.insert(name.to_string(), e.clone());
        Ok(e)
    }

    /// The shape of `t`, adding the shapes of the structs and enums it holds first.
    fn shape_of(&mut self, structs: &Structs, t: &SemaTy) -> Result<Shape, String> {
        match t {
            SemaTy::Struct(n) => Ok(Shape::Struct(self.add(structs, n)?)),
            SemaTy::Enum(n) => Ok(Shape::Enum(self.add_enum(structs, n)?)),
            SemaTy::Array(n, e) => Ok(Shape::Array(
                u32::try_from(*n).map_err(|_| format!("an array of {} elements", n))?,
                Box::new(self.shape_of(structs, e)?),
            )),
            other @ (SemaTy::Int(_) | SemaTy::Bool | SemaTy::Unit | SemaTy::Any) => {
                Shape::of(other, self)
            }
        }
    }
}

impl Shape {
    /// The struct this shape is, or `what`'s error.
    fn as_struct(&self, what: &str) -> Result<&Arc<StructShape>, String> {
        match self {
            Shape::Struct(s) => Ok(s),
            Shape::Int(_) | Shape::Bool | Shape::Array(..) | Shape::Enum(_) | Shape::Unit => {
                Err(format!("{} that is not a struct", what))
            }
        }
    }

    /// The enum this shape is, or `what`'s error.
    fn as_enum(&self, what: &str) -> Result<&Arc<EnumShape>, String> {
        match self {
            Shape::Enum(e) => Ok(e),
            Shape::Int(_) | Shape::Bool | Shape::Array(..) | Shape::Struct(_) | Shape::Unit => {
                Err(format!("{} that is not an enum", what))
            }
        }
    }

    /// The length and element shape of the array this shape is, or `what`'s error.
    fn as_array(&self, what: &str) -> Result<(u32, &Shape), String> {
        match self {
            Shape::Array(n, e) => Ok((*n, e)),
            Shape::Int(_) | Shape::Bool | Shape::Struct(_) | Shape::Enum(_) | Shape::Unit => {
                Err(format!("{} that is not an array", what))
            }
        }
    }

    pub fn of(t: &SemaTy, shapes: &Shapes) -> Result<Shape, String> {
        Ok(match t {
            SemaTy::Int(i) => Shape::Int(int_ty(i).unwrap_or(I64)),
            SemaTy::Bool => Shape::Bool,
            SemaTy::Array(n, e) => Shape::Array(
                u32::try_from(*n).map_err(|_| format!("an array of {} elements", n))?,
                // An empty literal's elements have no type, and need none: it holds none.
                Box::new(if *n == 0 && no_value(e) {
                    Shape::Unit
                } else {
                    Shape::of(e, shapes)?
                }),
            ),
            SemaTy::Struct(n) => Shape::Struct(
                shapes
                    .0
                    .get(n)
                    .cloned()
                    .ok_or_else(|| format!("a struct `{}` without a shape", n))?,
            ),
            SemaTy::Enum(n) => Shape::Enum(
                shapes
                    .1
                    .get(n)
                    .cloned()
                    .ok_or_else(|| format!("an enum `{}` without a shape", n))?,
            ),
            SemaTy::Unit => Shape::Unit,
            SemaTy::Any => return Err("a value of no type".into()),
        })
    }

    /// Bytes in memory.
    pub fn size(&self) -> u32 {
        match self {
            Shape::Int(t) => t.bits / 8,
            Shape::Bool => 1,
            Shape::Array(n, e) => n * e.size(),
            Shape::Struct(s) => s.size,
            Shape::Enum(e) => e.size,
            Shape::Unit => 0,
        }
    }

    /// Whether a value of this shape lives in memory, passed and returned by its address: an
    /// array or a struct.
    pub fn in_memory(&self) -> bool {
        matches!(self, Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_))
    }

    /// The MIR type of a value of this shape: an array or a struct is its address.
    pub fn ty(&self) -> Ty {
        match self {
            Shape::Int(t) => Ty::Int(mir(*t)),
            Shape::Bool => Ty::Bool,
            Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_) => Ty::Addr,
            Shape::Unit => Ty::Unit,
        }
    }
}

const I64: SemaIntTy = SemaIntTy {
    name: "i64",
    signed: true,
    bits: 64,
};
const USIZE: IntTy = IntTy::new(64, false);
/// The type a loop keeps its measure in, which is never negative there.
const MEASURE: IntTy = IntTy::new(128, false);

fn int_ty(i: &SemaInt) -> Option<SemaIntTy> {
    match i {
        SemaInt::One(t) | SemaInt::Mixed(Some(t)) => Some(*t),
        SemaInt::Literal | SemaInt::Mixed(None) => None,
    }
}

/// The MIR type of an integer type.
pub fn mir(t: SemaIntTy) -> IntTy {
    IntTy::new(t.bits as u8, t.signed)
}

/// The narrowest of `i64`, `u64`, `i128` and `u128` that holds every value from `lo` to `hi`.
/// The shape of the array type `t`, its integers at every depth of type `leaf`.
fn with_leaf(t: &SemaTy, leaf: SemaIntTy) -> Result<Shape, String> {
    match t {
        SemaTy::Array(n, elem) => {
            let n = u32::try_from(*n).map_err(|_| format!("an array of {} elements", n))?;
            Ok(Shape::Array(n, Box::new(with_leaf(elem, leaf)?)))
        }
        SemaTy::Int(_) => Ok(Shape::Int(leaf)),
        SemaTy::Bool | SemaTy::Struct(_) | SemaTy::Enum(_) | SemaTy::Unit | SemaTy::Any => {
            Err("an array literal whose elements are no integers".into())
        }
    }
}

fn machine(lo: Num, hi: Num) -> Option<IntTy> {
    [
        IntTy::new(64, true),
        IntTy::new(64, false),
        IntTy::new(128, true),
        IntTy::new(128, false),
    ]
    .into_iter()
    .find(|t| t.holds(lo) && t.holds(hi))
}

/// A lowered value.
#[derive(Clone, Copy, Debug)]
enum V {
    Int(NodeId, IntTy),
    Bool(NodeId),
    /// An array or a struct, in memory: its address.
    Mem(NodeId),
    Unit,
    /// Control left: a trap, a return, a `break` or a `continue`.
    Never,
}

impl V {
    /// Whether control left where this value would be.
    fn never(&self) -> bool {
        match self {
            V::Never => true,
            V::Int(..) | V::Bool(_) | V::Mem(_) | V::Unit => false,
        }
    }

    /// The node of a `bool`, none where control left; any other value is a lowering bug.
    fn bool_node(self, what: &str) -> Result<Option<NodeId>, String> {
        match self {
            V::Bool(n) => Ok(Some(n)),
            V::Never => Ok(None),
            V::Int(..) | V::Mem(_) | V::Unit => Err(format!("{} that is not a bool", what)),
        }
    }

    /// The address of a value in memory, none where control left; any other value is a
    /// lowering bug.
    fn mem_node(self, what: &str) -> Result<Option<NodeId>, String> {
        match self {
            V::Mem(a) => Ok(Some(a)),
            V::Never => Ok(None),
            V::Int(..) | V::Bool(_) | V::Unit => Err(format!("{} that is not in memory", what)),
        }
    }

    /// The node and type of an integer, none where control left; any other value is a
    /// lowering bug.
    fn int_node(self, what: &str) -> Result<Option<(NodeId, IntTy)>, String> {
        match self {
            V::Int(n, t) => Ok(Some((n, t))),
            V::Never => Ok(None),
            V::Bool(_) | V::Mem(_) | V::Unit => Err(format!("{} that is not an integer", what)),
        }
    }
}

/// Where a binding's value is: an SSA value, or memory of a shape.
#[derive(Clone, Debug)]
enum Local {
    Value(V),
    Place(NodeId, Shape),
}

/// How a function is called: its MIR function, its parameters as `(mut, shape)`, and its
/// result.
#[derive(Clone, Debug)]
struct Sig {
    func: FuncId,
    params: Vec<(bool, Shape)>,
    result: Shape,
}

/// A declared type: a type node in the tree it lives in, a function's or a struct's.
type Declared<'t> = Option<(&'t Tree, TypeId)>;

/// An argument of a call: a value with the span it is narrowed at and its expression's shape,
/// or for a `mut` parameter the address of a place, with its declared type.
#[derive(Clone, Debug)]
enum Arg<'t> {
    Value(V, Span, Shape),
    Place(NodeId, Shape, Declared<'t>, Span),
}

/// Lowers every function of `m` that has a body, and gives each its entry.
pub fn lower(m: &CheckedModule) -> Result<Lowered, String> {
    let shapes = Shapes::of(&m.structs)?;
    let mut sigs = Vec::new();
    let mut count = 0u32;
    for f in &m.functions {
        sigs.push(match f {
            Some(f) => {
                let tree = &f.syntax.tree;
                let params = f
                    .syntax
                    .params
                    .iter()
                    .map(|p| Ok((p.is_mut, Shape::of(&SemaTy::of(tree, p.ty), &shapes)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                let result = match f.syntax.result {
                    Some(t) => Shape::of(&SemaTy::of(tree, t), &shapes)?,
                    None => Shape::Unit,
                };
                count += 1;
                Some(Sig {
                    func: FuncId(count - 1),
                    params,
                    result,
                })
            }
            None => None,
        });
    }
    let mut module = cfg::Module::default();
    for (i, f) in m.functions.iter().enumerate() {
        if let Some(f) = f {
            let lowered = Lower::new(m, &sigs, &shapes, i, false)?
                .function()
                .map_err(|e| format!("fn {}: {}", f.syntax.name, e))?;
            module.functions.push(lowered);
        }
    }
    let mut entries = Vec::new();
    for (i, f) in m.functions.iter().enumerate() {
        if let (Some(f), Some(sig)) = (f, &sigs[i]) {
            let func = FuncId(module.functions.len() as u32);
            let entry = Lower::new(m, &sigs, &shapes, i, true)?
                .entry()
                .map_err(|e| format!("entry of {}: {}", f.syntax.name, e))?;
            module.functions.push(entry);
            entries.push(Entry {
                name: f.syntax.name.clone(),
                func,
                params: sig.params.clone(),
                result: sig.result.clone(),
                at: Loc {
                    line: f.syntax.name_span.line,
                    col: f.syntax.name_span.col,
                },
            });
        }
    }
    cfg::verify(&module)?;
    Ok(Lowered { module, entries })
}

/// The MIR parameters of a function of `sig`: the address of a result in memory first, then
/// each parameter, a `mut` one or one in memory as an address; and its MIR result.
fn mir_params(sig: &Sig) -> (Vec<Ty>, Option<Ty>) {
    let mut params = Vec::new();
    if sig.result.in_memory() {
        params.push(Ty::Addr);
    }
    for (is_mut, shape) in &sig.params {
        params.push(if *is_mut { Ty::Addr } else { shape.ty() });
    }
    let result = match &sig.result {
        Shape::Int(_) | Shape::Bool => Some(sig.result.ty()),
        Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_) | Shape::Unit => None,
    };
    (params, result)
}

/// The context expressions are lowered in: a function's syntax, the static ranges of its
/// expressions and where its bindings are. A callee's contract clauses are lowered in the
/// callee's context, its parameters bound to the call's arguments.
#[derive(Clone)]
struct Scope<'a> {
    index: usize,
    f: &'a CheckedFn,
    iv: Vec<Option<Interval>>,
    locals: Vec<Option<Local>>,
}

impl<'a> Scope<'a> {
    fn new(m: &'a CheckedModule, index: usize) -> Result<Scope<'a>, String> {
        let f = m.functions[index]
            .as_ref()
            .ok_or("a call of a function without a body")?;
        Ok(Scope {
            index,
            f,
            iv: wide::intervals(&f.syntax, &f.resolution, &f.typing),
            locals: vec![None; f.resolution.bindings.len()],
        })
    }
}

/// Where Tier 1 records the obligation of a check: its place, or any place for a written
/// invariant, whose conjuncts it records each at its own; its tag; and whether it is one of a
/// call's own checks, made in the callee's scope but recorded in the caller's.
#[derive(Clone, Copy)]
struct Key {
    at: Option<Span>,
    tag: Tag,
    call: bool,
}

impl Key {
    fn at(at: Span, tag: Tag) -> Key {
        Key {
            at: Some(at),
            tag,
            call: false,
        }
    }

    /// A call's own check, at the call or an argument.
    fn call(at: Span, tag: Tag) -> Key {
        Key {
            at: Some(at),
            tag,
            call: true,
        }
    }

    /// A written invariant, `clause`.
    fn invariant(clause: Span) -> Key {
        Key {
            at: None,
            tag: Tag::Clause(clause),
            call: false,
        }
    }
}

/// A step of a place from its root binding: an index, with its value, the span of its indexing,
/// where it traps, and the span of the index itself, where Tier 1 records it; or a field, by
/// its index in the struct's declaration.
#[derive(Clone, Copy, Debug)]
enum Step {
    Index(V, Span, Span),
    Field(usize),
}

/// Whether a place's steps end in an index of no value, after which the place is never reached.
fn reaches_never(steps: &[Step]) -> bool {
    steps
        .last()
        .is_some_and(|s| matches!(s, Step::Index(V::Never, _, _)))
}

/// Whether a value of type `t` never exists: no type, which a value that leaves has, or an
/// array of at least one such element, which a literal of only such elements has
/// (`core-calculus.md`, item 6). An empty literal holds no element, and is a value.
fn no_value(t: &SemaTy) -> bool {
    match t {
        SemaTy::Any => true,
        SemaTy::Array(n, elem) => *n > 0 && no_value(elem),
        SemaTy::Int(_) | SemaTy::Bool | SemaTy::Struct(_) | SemaTy::Enum(_) | SemaTy::Unit => false,
    }
}

/// One function's lowering.
struct Lower<'a> {
    m: &'a CheckedModule,
    sigs: &'a [Option<Sig>],
    shapes: &'a Shapes,
    /// Whether this is an entry, whose arguments come from outside the program: a struct's
    /// fields are checked against their refinements there, which a value made inside the
    /// program always holds.
    entry: bool,
    s: Scope<'a>,
    b: Builder,
    /// Whether each block has its terminator; one that never gets one is unreachable.
    ended: Vec<bool>,
    /// The enclosing loops' heads and exits.
    loops: Vec<(BlockId, BlockId)>,
    /// Where an array result goes.
    sret: Option<NodeId>,
    /// This function's `decreases` measure on entry, for its recursive calls.
    measure: Option<Vec<V>>,
    /// What Tier 1 recorded in the function, which tells the checks it proves; nothing for an
    /// entry, whose arguments come from outside the program.
    obligations: &'a [Obligation],
    /// Whether the lowering is making a call's checks in its callee's scope, whose checks are
    /// never left out.
    in_callee: bool,
}

impl<'a> Lower<'a> {
    fn new(
        m: &'a CheckedModule,
        sigs: &'a [Option<Sig>],
        shapes: &'a Shapes,
        index: usize,
        entry: bool,
    ) -> Result<Lower<'a>, String> {
        let s = Scope::new(m, index)?;
        let sig = sigs[index]
            .as_ref()
            .ok_or("a function without a signature")?;
        let (params, result) = mir_params(sig);
        let obligations: &'a [Obligation] = if entry { &[] } else { &s.f.obligations };
        let decl = s.f.syntax.name_span;
        let b = Builder::declared_at(
            &s.f.syntax.name,
            &params,
            result,
            Loc {
                line: decl.line,
                col: decl.col,
            },
        );
        Ok(Lower {
            m,
            sigs,
            shapes,
            entry,
            s,
            b,
            ended: vec![false],
            loops: Vec::new(),
            sret: None,
            measure: None,
            obligations,
            in_callee: false,
        })
    }

    /// Whether Tier 1 proves the check of `kind` it records under `key`: some obligation has
    /// that kind, place and tag, and every one that does is proven. In a callee's scope, only a
    /// call's own checks; in an entry, which has no obligations, none.
    fn proven(&self, kind: ObligationKind, key: Key) -> bool {
        if self.in_callee && !key.call {
            return false;
        }
        let mut found = self
            .obligations
            .iter()
            .filter(|o| o.kind == kind && key.at.is_none_or(|at| o.span == at) && o.tag == key.tag)
            .peekable();
        found.peek().is_some() && found.all(|o| o.proven)
    }

    fn sig_of(&self, index: usize) -> Result<&'a Sig, String> {
        self.sigs[index]
            .as_ref()
            .ok_or_else(|| "a call of a function without a body".to_string())
    }

    fn tree(&self) -> &'a Tree {
        &self.s.f.syntax.tree
    }

    fn at(&mut self, span: Span) {
        self.b.at(span.line, span.col);
    }

    fn block(&mut self, params: &[Ty]) -> BlockId {
        let id = self.b.block(params);
        self.ended.push(false);
        id
    }

    fn end(&mut self, term: Term) {
        self.ended[self.b.current().0 as usize] = true;
        self.b.end(term);
    }

    /// Continues in a new block nothing jumps to, after control has left.
    fn dead(&mut self) -> V {
        let dead = self.block(&[]);
        self.b.switch(dead);
        V::Never
    }

    /// Ends the current block in a trap at `span`.
    fn trap(&mut self, kind: &str, text: &str, span: Span) -> V {
        self.at(span);
        let site = self.b.trap_site(kind, &[text]);
        self.end(Term::Trap(site));
        self.dead()
    }

    fn jump(&mut self, to: BlockId, args: Vec<NodeId>) {
        self.end(Term::Jump(to, args));
    }

    fn int(&mut self, t: IntTy, n: Num) -> NodeId {
        self.b.int(t, n)
    }

    fn inst(&mut self, op: Op, ty: Ty, args: &[NodeId]) -> NodeId {
        self.b.inst(op, ty, args)
    }

    fn bool(&mut self, b: bool) -> NodeId {
        self.inst(Op::Bool(b), Ty::Bool, &[])
    }

    /// Traps at `span` with the pieces of `text` around `values` unless `cond` holds.
    fn check(&mut self, cond: NodeId, kind: &str, text: &[&str], values: &[NodeId], span: Span) {
        self.at(span);
        let site = self.b.trap_site(kind, text);
        let mut args = vec![cond];
        args.extend_from_slice(values);
        self.inst(Op::Check { trap: site }, Ty::Unit, &args);
    }

    fn finish(mut self) -> cfg::Function {
        let unreachable = self.b.trap_site(
            "unreachable",
            &["unreachable reached: control left the code the checker allows"],
        );
        for i in 0..self.ended.len() {
            if !self.ended[i] {
                self.b.switch(BlockId(i as u32));
                self.b.end(Term::Trap(unreachable));
            }
        }
        self.b.finish()
    }

    // -- functions -------------------------------------------------------------------------

    fn function(mut self) -> Result<cfg::Function, String> {
        let sig = self.sig_of(self.s.index)?;
        let mut params = self.b.params(BlockId(0)).into_iter();
        if sig.result.in_memory() {
            self.sret = params.next();
        }
        for (i, (is_mut, shape)) in sig.params.iter().enumerate() {
            let p = params.next().ok_or("too few parameters")?;
            self.s.locals[i] = Some(match (is_mut, shape) {
                (false, Shape::Int(t)) => Local::Value(V::Int(p, mir(*t))),
                (false, Shape::Bool) => Local::Value(V::Bool(p)),
                (_, shape) => Local::Place(p, shape.clone()),
            });
        }
        // The measure on entry, unchecked: the caller checked it.
        let clauses = self.clauses(ContractKind::Decreases);
        if !clauses.is_empty() {
            let mut values = Vec::new();
            for c in clauses {
                values.push(self.expr(c)?);
            }
            self.measure = Some(values);
        }
        let tree = self.tree();
        match self.s.f.syntax.body {
            Some(Body::Expr(e)) => {
                let v = self.expr(e)?;
                let shape = self.shape(e)?;
                self.ret(v, shape, tree.expr(e).span)?;
            }
            Some(Body::Block(b)) => {
                let block = tree.block(b);
                let span = block
                    .stmts
                    .last()
                    .map_or(block.span, |&s| tree.stmt(s).span);
                let shape = match block.stmts.last().map(|&s| &tree.stmt(s).kind) {
                    Some(StmtKind::Expr(e)) => self.shape(*e)?,
                    _ => Shape::Unit,
                };
                let v = self.body(b)?;
                self.ret(v, shape, span)?;
            }
            None => {
                self.ret(V::Unit, Shape::Unit, self.s.f.syntax.name_span)?;
            }
        }
        Ok(self.finish())
    }

    fn clauses(&self, kind: ContractKind) -> Vec<ExprId> {
        self.s
            .f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| c.expr)
            .collect()
    }

    /// The entry: a function taking what the lowered one takes, each argument already in its
    /// parameter's type, which checks the rest as a call does, at the function's name, and calls
    /// it.
    fn entry(mut self) -> Result<cfg::Function, String> {
        let span = self.s.f.syntax.name_span;
        let sig = self.sig_of(self.s.index)?;
        let mut params = self.b.params(BlockId(0)).into_iter();
        let sret = if sig.result.in_memory() {
            params.next()
        } else {
            None
        };
        let mut args = Vec::new();
        for (i, (is_mut, shape)) in sig.params.iter().enumerate() {
            let p = params.next().ok_or("too few parameters")?;
            let declared = Some((self.tree(), self.s.f.syntax.params[i].ty));
            // Each parameter holds its argument here too, as in the body, so that a refinement
            // bound naming one reads the value passed for it.
            self.s.locals[i] = Some(match (is_mut, shape) {
                (false, Shape::Int(t)) => Local::Value(V::Int(p, mir(*t))),
                (false, Shape::Bool) => Local::Value(V::Bool(p)),
                (_, shape) => Local::Place(p, shape.clone()),
            });
            args.push(match (is_mut, shape) {
                (false, Shape::Int(t)) => Arg::Value(V::Int(p, mir(*t)), span, shape.clone()),
                (false, Shape::Bool) => Arg::Value(V::Bool(p), span, shape.clone()),
                (false, _) => Arg::Value(V::Mem(p), span, shape.clone()),
                (true, shape) => Arg::Place(p, shape.clone(), declared, span),
            });
        }
        let v = self.call(self.s.index, args, span, sret, false)?;
        let term = match v {
            V::Int(n, _) | V::Bool(n) => Term::Return(Some(n)),
            V::Mem(_) | V::Unit | V::Never => Term::Return(None),
        };
        self.end(term);
        let mut f = self.finish();
        f.name = format!("enter {}", f.name);
        Ok(f)
    }

    /// Returns `v`, whose expression has `shape`, from this function: narrowed into the result
    /// type at `span`, checked against its refinement, and against each `ensures` at `span`.
    fn ret(&mut self, v: V, shape: Shape, span: Span) -> Result<(), String> {
        if v.never() {
            return Ok(());
        }
        let result = self.sig_of(self.s.index)?.result.clone();
        let declared = self.s.f.syntax.result.map(|t| (self.tree(), t));
        let v = self.land(v, &shape, &result, declared, span)?;
        let tree = self.tree();
        for c in self.clauses(ContractKind::Ensures) {
            // Each `ensures` clause has its own `result` binding, declared at the clause.
            let clause = tree.expr(c).span;
            if let Some(id) = self
                .s
                .f
                .resolution
                .bindings
                .iter()
                .position(|b| b.kind == BindingKind::Result && b.span == clause)
            {
                self.s.locals[id] = Some(match v {
                    V::Mem(a) => Local::Place(a, result.clone()),
                    v @ (V::Int(..) | V::Bool(_) | V::Unit | V::Never) => Local::Value(v),
                });
            }
            let text = format!("`ensures {}` is false at this return", tree.render_expr(c));
            let key = Key::at(span, Tag::Clause(tree.expr(c).span));
            self.clause(c, ("ensures", ObligationKind::Ensures), &text, span, key)?;
        }
        match v {
            V::Int(n, _) | V::Bool(n) => self.end(Term::Return(Some(n))),
            V::Mem(a) => {
                let to = self.sret.ok_or("an array result with nowhere to go")?;
                self.at(span);
                self.inst(
                    Op::Copy {
                        bytes: result.size(),
                    },
                    Ty::Unit,
                    &[to, a],
                );
                self.end(Term::Return(None));
            }
            V::Unit | V::Never => self.end(Term::Return(None)),
        }
        self.dead();
        Ok(())
    }

    /// Traps at `span` with `text` unless the clause `c`, of the current scope, holds, or
    /// Tier 1 proves it under `key`. The clause is evaluated either way, so that a trap inside
    /// it stays where the reference interpreter has it.
    fn clause(
        &mut self,
        c: ExprId,
        (kind, obligation): (&str, ObligationKind),
        text: &str,
        span: Span,
        key: Key,
    ) -> Result<(), String> {
        if let Some(cond) = self.expr(c)?.bool_node("a clause")? {
            if !self.proven(obligation, key) {
                self.check(cond, kind, &[text], &[], span);
            }
        }
        Ok(())
    }

    // -- statements ------------------------------------------------------------------------

    /// The value of a block: its last statement's, when that is an expression.
    fn body(&mut self, b: AstBlock) -> Result<V, String> {
        let tree = self.tree();
        let stmts = &tree.block(b).stmts;
        let mut last = V::Unit;
        for (i, &s) in stmts.iter().enumerate() {
            last = self.stmt(s, i + 1 == stmts.len())?;
            if last.never() {
                return Ok(V::Never);
            }
        }
        Ok(last)
    }

    fn stmt(&mut self, s: StmtId, last: bool) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.stmt(s).span;
        self.at(span);
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => {
                let v = self.expr(*e)?;
                return Ok(if last || v.never() { v } else { V::Unit });
            }
            StmtKind::Discard(e) => {
                if self.expr(*e)?.never() {
                    return Ok(V::Never);
                }
            }
            StmtKind::Binding { ty, init, .. } => {
                let Some(id) = self.s.f.resolution.stmts[s.0 as usize] else {
                    return Ok(V::Unit);
                };
                let id = id.0 as usize;
                let v = self.expr(*init)?;
                // An initializer that leaves, of no type when every branch of it does, gives
                // the binding nothing to hold (`core-calculus.md`, item 6).
                if v.never() {
                    return Ok(V::Never);
                }
                let shape = Shape::of(&self.s.f.typing.bindings[id], self.shapes)?;
                let from = self.shape(*init)?;
                let declared = ty.map(|t| (tree, t));
                let v = self.land(v, &from, &shape, declared, tree.expr(*init).span)?;
                if v.never() {
                    return Ok(V::Never);
                }
                let is_var = self.s.f.resolution.bindings[id].kind == BindingKind::Var;
                let local = match (v, is_var) {
                    (V::Mem(a), _) => Local::Place(self.copy(a, &shape), shape),
                    (v, false) => Local::Value(v),
                    (v, true) => {
                        let slot = self.inst(
                            Op::Slot {
                                bytes: shape.size(),
                            },
                            Ty::Addr,
                            &[],
                        );
                        self.store_shaped(slot, v, &shape);
                        Local::Place(slot, shape)
                    }
                };
                self.s.locals[id] = Some(local);
            }
            StmtKind::Assign { place, op, value } => return self.assign(*place, *op, *value),
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => return self.while_loop(*cond, invariants, decreases, *body),
            StmtKind::Guard { cond, otherwise } => {
                let Some(c) = self.expr(*cond)?.bool_node("a condition")? else {
                    return Ok(V::Never);
                };
                let pass = self.block(&[]);
                let fail = self.block(&[]);
                self.end(Term::Branch {
                    cond: c,
                    then: (pass, vec![]),
                    otherwise: (fail, vec![]),
                });
                self.b.switch(fail);
                // The `else` leaves, so its end is never reached.
                self.body(*otherwise)?;
                self.b.switch(pass);
            }
            StmtKind::Return(value) => {
                match value {
                    Some(e) => {
                        let v = self.expr(*e)?;
                        let shape = self.shape(*e)?;
                        self.ret(v, shape, tree.expr(*e).span)?;
                    }
                    None => self.ret(V::Unit, Shape::Unit, span)?,
                }
                return Ok(V::Never);
            }
            StmtKind::Break | StmtKind::Continue => {
                let (head, exit) = *self.loops.last().ok_or("a jump outside a loop")?;
                let to = if matches!(tree.stmt(s).kind, StmtKind::Break) {
                    exit
                } else {
                    head
                };
                self.jump(to, vec![]);
                return Ok(self.dead());
            }
        }
        Ok(V::Unit)
    }

    fn while_loop(
        &mut self,
        cond: ExprId,
        invariants: &[ExprId],
        decreases: &[ExprId],
        body: AstBlock,
    ) -> Result<V, String> {
        let tree = self.tree();
        // The measure of the round before, in memory, and whether there was one.
        let before = if decreases.is_empty() {
            None
        } else {
            let seen = self.inst(Op::Slot { bytes: 1 }, Ty::Addr, &[]);
            let no = self.bool(false);
            self.inst(Op::Store, Ty::Unit, &[seen, no]);
            let slots: Vec<NodeId> = decreases
                .iter()
                .map(|_| self.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]))
                .collect();
            Some((seen, slots))
        };
        let head = self.block(&[]);
        let round = self.block(&[]);
        let exit = self.block(&[]);
        self.jump(head, vec![]);
        self.b.switch(head);
        for &i in invariants {
            let text = format!(
                "`invariant {}` is false at the loop head",
                tree.render_expr(i)
            );
            let (at, key) = (tree.expr(i).span, Key::invariant(tree.expr(i).span));
            self.clause(i, ("invariant", ObligationKind::Invariant), &text, at, key)?;
        }
        let Some(c) = self.expr(cond)?.bool_node("a condition")? else {
            return Ok(V::Never);
        };
        self.end(Term::Branch {
            cond: c,
            then: (round, vec![]),
            otherwise: (exit, vec![]),
        });
        self.b.switch(round);
        if let (Some((seen, slots)), Some(&first)) = (before, decreases.first()) {
            let now = self.measure(decreases, None)?;
            // A measure of any integer type fits u128 once it is not negative.
            let prev: Vec<V> = slots
                .iter()
                .map(|s| V::Int(self.inst(Op::Load, Ty::Int(MEASURE), &[*s]), MEASURE))
                .collect();
            let smaller = self.lex_less(&now, &prev)?;
            let had = self.inst(Op::Load, Ty::Bool, &[seen]);
            let first_round = self.inst(Op::Not, Ty::Bool, &[had]);
            let ok = self.inst(Op::Bits(BitOp::Or), Ty::Bool, &[first_round, smaller]);
            let text = format!(
                "`decreases {}` is negative or did not decrease",
                tree.render_expr(first)
            );
            let fspan = tree.expr(first).span;
            if !self.proven(ObligationKind::Decreases, Key::at(fspan, Tag::Smaller)) {
                self.check(ok, "decreases", &[&text], &[], fspan);
            }
            let yes = self.bool(true);
            self.inst(Op::Store, Ty::Unit, &[seen, yes]);
            for (v, s) in now.iter().zip(&slots) {
                if let Some((n, t)) = v.int_node("a measure")? {
                    let w = self.cast(n, t, MEASURE);
                    self.inst(Op::Store, Ty::Unit, &[*s, w]);
                }
            }
        }
        self.loops.push((head, exit));
        let v = self.body(body)?;
        self.loops.pop();
        if !v.never() {
            self.jump(head, vec![]);
        }
        self.b.switch(exit);
        Ok(V::Unit)
    }

    /// The values of a measure, each checked not to be negative at its clause, unless Tier 1
    /// proves it: at the clause in a loop, at the call `call` for a callee's.
    fn measure(&mut self, exprs: &[ExprId], call: Option<Span>) -> Result<Vec<V>, String> {
        let tree = self.tree();
        let mut values = Vec::new();
        for &e in exprs {
            let v = self.expr(e)?;
            let clause = Tag::Clause(tree.expr(e).span);
            let key = call.map_or(Key::at(tree.expr(e).span, clause), |at| {
                Key::call(at, clause)
            });
            let proven = self.proven(ObligationKind::Decreases, key);
            if let (Some((n, t)), false) = (v.int_node("a measure")?, proven) {
                let zero = self.int(t, Num::ZERO);
                let ok = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[n, zero]);
                let text = format!(
                    "`decreases {}` is negative or did not decrease",
                    tree.render_expr(e)
                );
                self.check(ok, "decreases", &[&text], &[], tree.expr(e).span);
            }
            values.push(v);
        }
        Ok(values)
    }

    /// Whether `a` is smaller than `b` in lexicographic order, both of one length.
    fn lex_less(&mut self, a: &[V], b: &[V]) -> Result<NodeId, String> {
        let mut result = self.bool(false);
        // From the last part back: a < b when a0 < b0, or a0 == b0 and the rest is smaller.
        for (x, y) in a.iter().zip(b).rev() {
            let measure = |v: &V| {
                (*v).int_node("a measure")?
                    .ok_or_else(|| "a measure of no value".to_string())
            };
            let ((x, tx), (y, ty)) = (measure(x)?, measure(y)?);
            let lt = self.compare(CmpOp::Lt, (x, tx), (y, ty))?;
            let eq = self.compare(CmpOp::Eq, (x, tx), (y, ty))?;
            let rest = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[eq, result]);
            result = self.inst(Op::Bits(BitOp::Or), Ty::Bool, &[lt, rest]);
        }
        Ok(result)
    }

    fn assign(&mut self, place: ExprId, op: AssignOp, value: ExprId) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.expr(value).span;
        let (root, steps) = self.place_path(place)?;
        if reaches_never(&steps) {
            return Ok(V::Never);
        }
        let shape = self.shape(place)?;
        let declared = self.place_type(root, &steps)?;
        let (new, from) = match op {
            AssignOp::Set => (self.expr(value)?, self.shape(value)?),
            AssignOp::Compound(op) => {
                let at = self.address(root, &steps)?;
                let current = self.load(at, &shape);
                let rhs = self.expr(value)?;
                let iv = Self::place_interval(&shape);
                let viv = self.interval(value);
                let combined = match (iv, viv) {
                    (Some(a), Some(b)) => wide::combine(op, a, b),
                    _ => None,
                };
                let v = self.binary(op, (current, iv, place), (rhs, viv, value), combined, span)?;
                (v, shape.clone())
            }
        };
        let new = self.land(new, &from, &shape, declared, span)?;
        if new.never() {
            return Ok(V::Never);
        }
        let at = self.address(root, &steps)?;
        self.store_shaped(at, new, &shape);
        Ok(V::Unit)
    }

    /// The root binding of a place and its steps, its indexes evaluated left to right, each
    /// with the span of its indexing and of the index itself. An index of no value, such as a
    /// `todo`, ends them: nothing after it is reached ([`reaches_never`]).
    fn place_path(&mut self, e: ExprId) -> Result<(usize, Vec<Step>), String> {
        let tree = self.tree();
        match &tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (root, mut steps) = self.place_path(*base)?;
                if reaches_never(&steps) {
                    return Ok((root, steps));
                }
                let i = self.expr(*index)?;
                steps.push(Step::Index(i, tree.expr(e).span, tree.expr(*index).span));
                Ok((root, steps))
            }
            ExprKind::Field { base, name } => {
                let (root, mut steps) = self.place_path(*base)?;
                if reaches_never(&steps) {
                    return Ok((root, steps));
                }
                let shape = self.shape(*base)?;
                let s = shape.as_struct("a field of a value")?;
                let (k, _) = s.field(name).ok_or("a field the struct lacks")?;
                steps.push(Step::Field(k));
                Ok((root, steps))
            }
            ExprKind::Name(_) => Ok((
                self.s.f.resolution.names[e.0 as usize]
                    .map(|b| b.0 as usize)
                    .ok_or("a place without a binding")?,
                Vec::new(),
            )),
            // The checker passes as a place only a binding, an element, or a field of a struct
            // (`E0205`, `E0206`).
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => Err("a place that is not a binding, an element or a field".into()),
        }
    }

    /// The declared type of a place: its root binding's, when it states one, down its indexes;
    /// a field's, in its struct's declaration, whatever its root states.
    fn place_type(&self, root: usize, steps: &[Step]) -> Result<Declared<'a>, String> {
        let f = self.s.f;
        let binding = f
            .resolution
            .bindings
            .get(root)
            .ok_or("a place without its binding")?;
        let mut declared = binding.ty.map(|t| (self.tree(), t));
        let mut ty = f
            .typing
            .bindings
            .get(root)
            .ok_or("a place without its binding's type")?
            .clone();
        for step in steps {
            match step {
                Step::Index(..) => {
                    let elem = match ty {
                        SemaTy::Array(_, elem) => *elem,
                        SemaTy::Int(_)
                        | SemaTy::Bool
                        | SemaTy::Struct(_)
                        | SemaTy::Enum(_)
                        | SemaTy::Unit
                        | SemaTy::Any => {
                            return Err("an element of a place that is not an array".into())
                        }
                    };
                    // A binding declared without a type has none down its elements either.
                    declared = match declared {
                        None => None,
                        Some((tree, t)) => match &tree.ty(t).kind {
                            TypeKind::Array { elem, .. } => Some((tree, *elem)),
                            TypeKind::Int { .. }
                            | TypeKind::Bool
                            | TypeKind::Named(_)
                            | TypeKind::Enum(_)
                            | TypeKind::Error => {
                                return Err("an array place declared of another type".into())
                            }
                        },
                    };
                    ty = elem;
                }
                Step::Field(k) => {
                    let s = match ty {
                        SemaTy::Struct(s) => s,
                        SemaTy::Int(_)
                        | SemaTy::Bool
                        | SemaTy::Array(..)
                        | SemaTy::Enum(_)
                        | SemaTy::Unit
                        | SemaTy::Any => {
                            return Err("a field of a place that is not a struct".into())
                        }
                    };
                    let def = self
                        .m
                        .structs
                        .get(&s)
                        .ok_or("a struct without a declaration")?;
                    let field = def.fields.get(*k).ok_or("a field the struct lacks")?;
                    declared = Some((def.tree(), field.decl));
                    ty = field.ty.clone();
                }
            }
        }
        Ok(declared)
    }

    /// The address of a place, each index checked against its array's length.
    fn address(&mut self, root: usize, steps: &[Step]) -> Result<NodeId, String> {
        let local = self.s.locals[root].clone().ok_or("a place never bound")?;
        let (mut at, mut shape) = match local {
            Local::Place(at, shape) => (at, shape),
            Local::Value(_) => return Err("a place that is not in memory".into()),
        };
        for step in steps {
            match (step, shape) {
                (Step::Index(i, span, key), Shape::Array(len, elem)) => {
                    at = self.element(at, len, &elem, *i, (*span, *key))?;
                    shape = *elem;
                }
                (Step::Field(k), Shape::Struct(s)) => {
                    let f = s.fields.get(*k).ok_or("a field the struct lacks")?;
                    at = self.field_addr(at, f.offset);
                    shape = f.shape.clone();
                }
                _ => return Err("a step into a value that has no such part".into()),
            }
        }
        Ok(at)
    }

    /// The address `offset` bytes past `base`, where a field lies.
    fn field_addr(&mut self, base: NodeId, offset: u32) -> NodeId {
        if offset == 0 {
            return base;
        }
        let k = self.int(USIZE, Num::new(false, u128::from(offset)));
        self.inst(Op::Elem { stride: 1 }, Ty::Addr, &[base, k])
    }

    /// The address of element `i` of an array of `len` elements of `elem` at `base`; an index
    /// outside it traps at `span`, unless Tier 1 proves at `key`, the index's own span, that it
    /// lies inside.
    fn element(
        &mut self,
        base: NodeId,
        len: u32,
        elem: &Shape,
        i: V,
        (span, key): (Span, Span),
    ) -> Result<NodeId, String> {
        let (i, t) = i.int_node("an index")?.ok_or("an index of no value")?;
        self.at(span);
        if !self.proven(ObligationKind::Index, Key::at(key, Tag::None)) {
            let zero = self.int(t, Num::ZERO);
            let ge = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[i, zero]);
            let n = Num::new(false, u128::from(len));
            let below = if t.holds(n) {
                let n = self.int(t, n);
                self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n])
            } else {
                self.bool(true)
            };
            let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
            let after = format!(" is outside an array of length {}", len);
            self.check(ok, "index-out-of-bounds", &["index ", &after], &[i], span);
        }
        let index = self.cast(i, t, USIZE);
        Ok(self.inst(
            Op::Elem {
                stride: elem.size(),
            },
            Ty::Addr,
            &[base, index],
        ))
    }

    // -- memory ----------------------------------------------------------------------------

    fn load(&mut self, at: NodeId, shape: &Shape) -> V {
        match shape {
            Shape::Int(t) => V::Int(self.inst(Op::Load, Ty::Int(mir(*t)), &[at]), mir(*t)),
            Shape::Bool => V::Bool(self.inst(Op::Load, Ty::Bool, &[at])),
            Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_) => V::Mem(at),
            Shape::Unit => V::Unit,
        }
    }

    fn store_shaped(&mut self, at: NodeId, v: V, shape: &Shape) {
        match v {
            V::Mem(a) => {
                self.inst(
                    Op::Copy {
                        bytes: shape.size(),
                    },
                    Ty::Unit,
                    &[at, a],
                );
            }
            V::Int(n, _) | V::Bool(n) => {
                self.inst(Op::Store, Ty::Unit, &[at, n]);
            }
            // `()` has no bytes, and no value never reaches a store.
            V::Unit | V::Never => {}
        }
    }

    /// A new slot holding a copy of the array or struct at `a`.
    fn copy(&mut self, a: NodeId, shape: &Shape) -> NodeId {
        let bytes = shape.size();
        let slot = self.inst(Op::Slot { bytes }, Ty::Addr, &[]);
        self.inst(Op::Copy { bytes }, Ty::Unit, &[slot, a]);
        slot
    }

    /// Runs `each` on every index of an array of `len` elements, in a loop.
    fn each(
        &mut self,
        len: u32,
        each: &mut dyn FnMut(&mut Lower<'a>, NodeId) -> Result<(), String>,
    ) -> Result<(), String> {
        let head = self.block(&[Ty::Int(USIZE)]);
        let round = self.block(&[]);
        let exit = self.block(&[]);
        let zero = self.int(USIZE, Num::ZERO);
        self.jump(head, vec![zero]);
        self.b.switch(head);
        let i = self.b.params(head)[0];
        let n = self.int(USIZE, Num::new(false, u128::from(len)));
        let more = self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n]);
        self.end(Term::Branch {
            cond: more,
            then: (round, vec![]),
            otherwise: (exit, vec![]),
        });
        self.b.switch(round);
        each(self, i)?;
        let one = self.int(USIZE, Num::new(false, 1));
        let next = self.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: None,
            },
            Ty::Int(USIZE),
            &[i, one],
        );
        self.jump(head, vec![next]);
        self.b.switch(exit);
        Ok(())
    }

    // -- landing ---------------------------------------------------------------------------

    /// A value, whose expression has shape `from`, landing in a place of shape `to` whose
    /// declared type is `declared`: narrowed into the place's integer types, then checked
    /// against the declared type's refinements, both at `span`.
    fn land(
        &mut self,
        v: V,
        from: &Shape,
        to: &Shape,
        declared: Declared<'a>,
        span: Span,
    ) -> Result<V, String> {
        let v = self.narrow(v, from, to, span, Some(span))?;
        if let Some(d) = declared {
            self.refined(v, to, d, span, Key::at(span, Tag::None))?;
        }
        Ok(v)
    }

    /// A value of shape `from` narrowed into `to`, elementwise for an array, trapping at
    /// `span`; `key` is where Tier 1 records the narrowing, when it does.
    fn narrow(
        &mut self,
        v: V,
        from: &Shape,
        to: &Shape,
        span: Span,
        key: Option<Span>,
    ) -> Result<V, String> {
        Ok(match (v, from, to) {
            (V::Int(n, t), _, Shape::Int(into)) => {
                V::Int(self.narrow_int(n, t, *into, span, key), mir(*into))
            }
            (V::Mem(a), Shape::Array(_, fe), Shape::Array(len, te)) => {
                // Elements of the same type stay where they are; of another type, they are
                // narrowed into a new array.
                if fe == te {
                    return Ok(V::Mem(a));
                }
                let slot = self.inst(Op::Slot { bytes: to.size() }, Ty::Addr, &[]);
                let (fe, te, len) = ((**fe).clone(), (**te).clone(), *len);
                self.each(len, &mut |l, i| {
                    let src = l.inst(Op::Elem { stride: fe.size() }, Ty::Addr, &[a, i]);
                    let dst = l.inst(Op::Elem { stride: te.size() }, Ty::Addr, &[slot, i]);
                    let x = l.load(src, &fe);
                    let x = l.narrow(x, &fe, &te, span, key)?;
                    l.store_shaped(dst, x, &te);
                    Ok(())
                })?;
                V::Mem(slot)
            }
            (v, _, _) => v,
        })
    }

    /// `n` of type `from` narrowed into `to`: a check that it fits, where it may not and Tier 1
    /// does not prove it at `key`, then a cast.
    fn narrow_int(
        &mut self,
        n: NodeId,
        from: IntTy,
        to: SemaIntTy,
        span: Span,
        key: Option<Span>,
    ) -> NodeId {
        let into = mir(to);
        let proven =
            key.is_some_and(|k| self.proven(ObligationKind::Range, Key::at(k, Tag::Into(to.name))));
        if from == into || proven {
            return self.cast(n, from, into);
        }
        let (lo, hi) = (into.min(), into.max());
        let mut parts = Vec::new();
        // A bound needs a check where the source type reaches past it.
        if !into.holds(from.min()) {
            let c = self.int(from, lo);
            parts.push(self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[n, c]));
        }
        if !into.holds(from.max()) {
            let c = self.int(from, hi);
            parts.push(self.inst(Op::Cmp(CmpOp::Le), Ty::Bool, &[n, c]));
        }
        if let Some(&first) = parts.first() {
            let ok = parts[1..].iter().fold(first, |a, b| {
                self.inst(Op::Bits(BitOp::And), Ty::Bool, &[a, *b])
            });
            let after = format!(" does not fit in `{}`", to.name);
            self.check(ok, "narrowing", &["", &after], &[n], span);
        }
        self.cast(n, from, into)
    }

    fn cast(&mut self, n: NodeId, from: IntTy, to: IntTy) -> NodeId {
        if from == to {
            n
        } else {
            self.inst(Op::Cast, Ty::Int(to), &[n])
        }
    }

    /// Checks a value of `shape` against the refinements of the declared type `d`, elementwise
    /// for an array, trapping at `span`, unless Tier 1 proves under `key` that it fits them; in
    /// an entry, fieldwise for a struct too.
    fn refined(
        &mut self,
        v: V,
        shape: &Shape,
        (tree, d): (&'a Tree, TypeId),
        span: Span,
        key: Key,
    ) -> Result<(), String> {
        // What is checked is defined once, by `refines`.
        if self.proven(ObligationKind::Refinement, key) || !self.refines(tree, d) {
            return Ok(());
        }
        let kind = &tree.ty(d).kind;
        // A value of no type is never checked: control left before it was given.
        match kind {
            TypeKind::Int {
                refinement: Some(r),
                ..
            } => match v.int_node("a refined integer")? {
                Some((x, t)) => self.in_range(x, t, r, span),
                None => Ok(()),
            },
            TypeKind::Array { elem, .. } => {
                let Some(a) = v.mem_node("a refined array")? else {
                    return Ok(());
                };
                let (len, es) = shape.as_array("a refined array")?;
                let (elem, es) = (*elem, es.clone());
                self.each(len, &mut |l, i| {
                    let at = l.inst(Op::Elem { stride: es.size() }, Ty::Addr, &[a, i]);
                    let x = l.load(at, &es);
                    l.refined(x, &es, (tree, elem), span, key)
                })
            }
            TypeKind::Named(name) => {
                let Some(a) = v.mem_node("a struct with refined fields")? else {
                    return Ok(());
                };
                let s = shape.as_struct("a struct with refined fields")?.clone();
                let def = self
                    .m
                    .structs
                    .get(name)
                    .ok_or("a struct without a declaration")?;
                for (fs, fd) in s.fields.iter().zip(&def.fields) {
                    if !self.refines(def.tree(), fd.decl) {
                        continue;
                    }
                    let at = self.field_addr(a, fs.offset);
                    let x = self.load(at, &fs.shape);
                    self.refined(x, &fs.shape, (def.tree(), fd.decl), span, key)?;
                }
                Ok(())
            }
            TypeKind::Enum(name) => {
                let Some(a) = v.mem_node("an enum with refined payload fields")? else {
                    return Ok(());
                };
                let s = shape
                    .as_enum("an enum with refined payload fields")?
                    .clone();
                self.refined_payload(a, name, &s, span, key)
            }
            // `refines` finds nothing to check in these.
            TypeKind::Int {
                refinement: None, ..
            }
            | TypeKind::Bool
            | TypeKind::Error => Ok(()),
        }
    }

    /// Whether a value of the declared type `t` has a refinement [`Self::refined`] checks: at the
    /// bottom of its arrays, or, in an entry, in its structs' fields.
    fn refines(&self, tree: &Tree, t: TypeId) -> bool {
        match &tree.ty(t).kind {
            TypeKind::Int { refinement, .. } => refinement.is_some(),
            TypeKind::Array { elem, .. } => self.refines(tree, *elem),
            TypeKind::Named(name) if self.entry => self
                .m
                .structs
                .get(name)
                .is_some_and(|d| d.fields.iter().any(|f| self.refines(d.tree(), f.decl))),
            TypeKind::Enum(name) if self.entry => self.m.structs.get_enum(name).is_some_and(|d| {
                d.variants
                    .iter()
                    .any(|v| v.fields.iter().any(|f| self.refines(d.tree(), f.decl)))
            }),
            TypeKind::Bool | TypeKind::Named(_) | TypeKind::Enum(_) | TypeKind::Error => false,
        }
    }

    /// In an entry, checks the refinements of the payload of the variant an enum at `a` holds,
    /// which a value from outside the program may not hold.
    fn refined_payload(
        &mut self,
        a: NodeId,
        name: &str,
        s: &EnumShape,
        span: Span,
        key: Key,
    ) -> Result<(), String> {
        let def = self
            .m
            .structs
            .get_enum(name)
            .ok_or("an enum without a declaration")?;
        let t = mir(s.tag);
        let tag = self.inst(Op::Load, Ty::Int(t), &[a]);
        for (k, (vs, vd)) in s.variants.iter().zip(&def.variants).enumerate() {
            if !vd.fields.iter().any(|f| self.refines(def.tree(), f.decl)) {
                continue;
            }
            let n = self.int(t, Num::new(false, k as u128));
            let is = self.inst(Op::Cmp(CmpOp::Eq), Ty::Bool, &[tag, n]);
            let check = self.block(&[]);
            let after = self.block(&[]);
            self.end(Term::Branch {
                cond: is,
                then: (check, vec![]),
                otherwise: (after, vec![]),
            });
            self.b.switch(check);
            for (fs, fd) in vs.fields.iter().zip(&vd.fields) {
                if !self.refines(def.tree(), fd.decl) {
                    continue;
                }
                let at = self.field_addr(a, fs.offset);
                let x = self.load(at, &fs.shape);
                self.refined(x, &fs.shape, (def.tree(), fd.decl), span, key)?;
            }
            self.jump(after, vec![]);
            self.b.switch(after);
        }
        Ok(())
    }

    fn in_range(&mut self, x: NodeId, t: IntTy, r: &Refinement, span: Span) -> Result<(), String> {
        let lo = self.bound(&r.lo)?;
        let hi = self.bound(&r.hi)?;
        let ge = self.compare(CmpOp::Ge, (x, t), lo)?;
        let below = self.compare(if r.inclusive { CmpOp::Le } else { CmpOp::Lt }, (x, t), hi)?;
        let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
        let dots = if r.inclusive { "..=" } else { ".." };
        self.check(
            ok,
            "refinement",
            &["", " lies outside the refinement `[", dots, "]`"],
            &[x, lo.0, hi.0],
            span,
        );
        Ok(())
    }

    /// The value of a refinement bound in the current scope.
    fn bound(&mut self, b: &Bound) -> Result<(NodeId, IntTy), String> {
        let slot = |l: &Lower, name: &str| {
            l.s.f
                .resolution
                .bindings
                .iter()
                .position(|binding| binding.name == name)
        };
        Ok(match b {
            Bound::Int { negative, value } => {
                let n = Num::new(*negative, *value);
                let t = machine(n, n).ok_or("a bound beyond 128 bits")?;
                (self.int(t, n), t)
            }
            Bound::Name(n) => match slot(self, n).and_then(|i| self.s.locals[i].clone()) {
                Some(Local::Value(V::Int(v, t))) => (v, t),
                // A bound names a `val` or a read-only parameter, never a place in memory.
                _ => return Err(format!("a bound `{}` that is not an integer value", n)),
            },
            Bound::Len(n) => match slot(self, n).and_then(|i| self.s.locals[i].clone()) {
                Some(Local::Place(_, Shape::Array(len, _))) => {
                    (self.int(USIZE, Num::new(false, u128::from(len))), USIZE)
                }
                _ => return Err(format!("a bound `{}.len` that is not an array's", n)),
            },
        })
    }

    // -- comparisons -----------------------------------------------------------------------

    /// Compares two integers of any types, as exact values.
    fn compare(
        &mut self,
        op: CmpOp,
        (a, ta): (NodeId, IntTy),
        (b, tb): (NodeId, IntTy),
    ) -> Result<NodeId, String> {
        if let Some(c) = common(ta, tb) {
            let a = self.cast(a, ta, c);
            let b = self.cast(b, tb, c);
            return Ok(self.inst(Op::Cmp(op), Ty::Bool, &[a, b]));
        }
        // A u128 against a signed type: a negative value is below every u128, and any other
        // fits u128.
        let (u, s, swapped) = if ta.signed {
            (b, a, true)
        } else {
            (a, b, false)
        };
        let (su, ss) = if swapped { (tb, ta) } else { (ta, tb) };
        let zero = self.int(ss, Num::ZERO);
        let negative = self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[s, zero]);
        let known = self.block(&[]);
        let cast = self.block(&[]);
        let join = self.block(&[Ty::Bool]);
        self.end(Term::Branch {
            cond: negative,
            then: (known, vec![]),
            otherwise: (cast, vec![]),
        });
        self.b.switch(known);
        // With s < u: u > s, u >= s and u != s hold; with the operands swapped, s < u.
        let holds = match op {
            CmpOp::Ne => true,
            CmpOp::Eq => false,
            CmpOp::Gt | CmpOp::Ge => !swapped,
            CmpOp::Lt | CmpOp::Le => swapped,
        };
        let r = self.bool(holds);
        self.jump(join, vec![r]);
        self.b.switch(cast);
        let s = self.cast(s, ss, su);
        let (x, y) = if swapped { (s, u) } else { (u, s) };
        let r = self.inst(Op::Cmp(op), Ty::Bool, &[x, y]);
        self.jump(join, vec![r]);
        self.b.switch(join);
        Ok(self.b.params(join)[0])
    }

    // -- expressions -----------------------------------------------------------------------

    /// The shape of `e`'s value. An expression of no type, such as a `todo` or an `if` whose
    /// branches all leave, gives no value, `V::Never`, and its shape is never used: it is
    /// given as `Unit`.
    fn shape(&self, e: ExprId) -> Result<Shape, String> {
        let shapes = self.shapes;
        match &self.s.f.typing.exprs[e.0 as usize] {
            SemaTy::Any => Ok(Shape::Unit),
            t @ SemaTy::Array(_, elem) if self.mixed_leaf(elem) => {
                // An array literal of integers of no one type keeps its elements exact until it
                // lands, in the machine type that holds them all (`wide.rs`); so do the elements
                // of an array literal of such array literals, at every depth.
                let tree = self.tree();
                if !tree.expr(e).kind.is_array_literal() {
                    return Shape::of(t, shapes);
                }
                let mut hull: Option<(Num, Num)> = None;
                self.leaf_hull(e, &mut hull)?;
                let m = match hull {
                    Some((lo, hi)) => machine(lo, hi),
                    None => Some(IntTy::new(64, true)),
                }
                .ok_or("an array literal beyond 128 bits")?;
                let name = format!("{}{}", if m.signed { "i" } else { "u" }, m.bits);
                let leaf = SemaIntTy::named(&name).ok_or("a machine type without a name")?;
                with_leaf(t, leaf)
            }
            t @ (SemaTy::Int(_)
            | SemaTy::Bool
            | SemaTy::Array(..)
            | SemaTy::Struct(_)
            | SemaTy::Enum(_)
            | SemaTy::Unit) => Shape::of(t, shapes),
        }
    }

    /// Widens `hull` to hold every integer the array literal `e` holds, through the array
    /// literals nested in it: each element's static range.
    fn leaf_hull(&self, e: ExprId, hull: &mut Option<(Num, Num)>) -> Result<(), String> {
        // An element of no type leaves before the literal holds a value (`core-calculus.md`,
        // item 6), so it widens nothing.
        if no_value(&self.s.f.typing.exprs[e.0 as usize]) {
            return Ok(());
        }
        let tree = self.tree();
        let (lo, hi) = match &tree.expr(e).kind {
            ExprKind::Array(items) => {
                for &i in items {
                    self.leaf_hull(i, hull)?;
                }
                return Ok(());
            }
            ExprKind::Repeat { value, .. } => return self.leaf_hull(*value, hull),
            // An array of another kind among them has a type of its own, which joins the
            // others' in a standard type or makes the literal `E0222` (`wide.rs`): never here.
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => {
                let r = self
                    .interval(e)
                    .ok_or("an element without a static range")?;
                (Num::new(r.lo.0, r.lo.1), Num::new(r.hi.0, r.hi.1))
            }
        };
        *hull = Some(match *hull {
            Some((l, h)) => (l.min(lo), h.max(hi)),
            None => (lo, hi),
        });
        Ok(())
    }

    /// Whether `t` is an integer of no one type, the element of an array literal whose elements
    /// have different integer types.
    fn mixed(&self, t: &SemaTy) -> bool {
        match t {
            SemaTy::Int(i) => int_ty(i).is_none(),
            SemaTy::Bool
            | SemaTy::Array(..)
            | SemaTy::Struct(_)
            | SemaTy::Enum(_)
            | SemaTy::Unit
            | SemaTy::Any => false,
        }
    }

    /// Whether `t` is such an integer, or an array whose elements are, at any depth.
    fn mixed_leaf(&self, t: &SemaTy) -> bool {
        match t {
            SemaTy::Array(_, elem) => self.mixed_leaf(elem),
            t @ (SemaTy::Int(_)
            | SemaTy::Bool
            | SemaTy::Struct(_)
            | SemaTy::Enum(_)
            | SemaTy::Unit
            | SemaTy::Any) => self.mixed(t),
        }
    }

    /// Whether the array literal `e` keeps its integers exact: they are of no one type, and
    /// its shape is the machine type that holds them all (`shape`).
    fn exact(&self, e: ExprId) -> bool {
        match &self.s.f.typing.exprs[e.0 as usize] {
            SemaTy::Array(_, t) => self.mixed_leaf(t),
            SemaTy::Int(_)
            | SemaTy::Bool
            | SemaTy::Struct(_)
            | SemaTy::Enum(_)
            | SemaTy::Unit
            | SemaTy::Any => false,
        }
    }

    /// The array literal of `items`, built in `shape`; `span` names the outermost literal.
    fn array_literal(
        &mut self,
        items: &[ExprId],
        shape: &Shape,
        exact: bool,
        span: Span,
    ) -> Result<V, String> {
        let (_, elem) = shape.as_array("an array literal")?;
        let slot = self.inst(
            Op::Slot {
                bytes: shape.size(),
            },
            Ty::Addr,
            &[],
        );
        for (k, &item) in items.iter().enumerate() {
            let v = self.literal_element(item, elem, exact, span)?;
            if v.never() {
                return Ok(V::Never);
            }
            let idx = self.int(USIZE, Num::new(false, k as u128));
            let at = self.inst(
                Op::Elem {
                    stride: elem.size(),
                },
                Ty::Addr,
                &[slot, idx],
            );
            self.store_shaped(at, v, elem);
        }
        Ok(V::Mem(slot))
    }

    /// The array literal `[value; n]`, built in `shape`.
    fn repeat_literal(
        &mut self,
        value: ExprId,
        shape: &Shape,
        exact: bool,
        span: Span,
    ) -> Result<V, String> {
        let (len, elem) = shape.as_array("an array literal")?;
        let v = self.literal_element(value, elem, exact, span)?;
        if v.never() {
            return Ok(V::Never);
        }
        let slot = self.inst(
            Op::Slot {
                bytes: shape.size(),
            },
            Ty::Addr,
            &[],
        );
        let (elem, len) = (elem.clone(), len);
        self.each(len, &mut |l, i| {
            let at = l.inst(
                Op::Elem {
                    stride: elem.size(),
                },
                Ty::Addr,
                &[slot, i],
            );
            l.store_shaped(at, v, &elem);
            Ok(())
        })?;
        Ok(V::Mem(slot))
    }

    /// An element of an array literal, in the literal's element shape `elem`. Where the
    /// literal keeps its integers exact, an integer is cast into `elem`, which holds it, and an
    /// array literal nested in it is built in `elem` itself, so that its integers are never
    /// narrowed on the way; any other element is narrowed into `elem`.
    fn literal_element(
        &mut self,
        item: ExprId,
        elem: &Shape,
        exact: bool,
        span: Span,
    ) -> Result<V, String> {
        let tree = self.tree();
        if exact && matches!(elem, Shape::Array(..)) {
            if let ExprKind::Array(items) = &tree.expr(item).kind {
                return self.array_literal(items, elem, true, span);
            }
            if let ExprKind::Repeat { value, .. } = &tree.expr(item).kind {
                return self.repeat_literal(*value, elem, true, span);
            }
        }
        let v = self.expr(item)?;
        let from = self.shape(item)?;
        let key = Some(tree.expr(item).span);
        Ok(match (exact, v, elem) {
            (true, V::Int(n, t), Shape::Int(to)) => {
                let into = mir(*to);
                V::Int(self.cast(n, t, into), into)
            }
            (_, v, _) => self.narrow(v, &from, elem, span, key)?,
        })
    }

    fn interval(&self, e: ExprId) -> Option<Interval> {
        self.s.iv.get(e.0 as usize).copied().flatten()
    }

    /// The range of a place of `shape`: its type's, which holds every value it can take.
    fn place_interval(shape: &Shape) -> Option<Interval> {
        let t = match shape {
            Shape::Int(t) => mir(*t),
            Shape::Bool | Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_) | Shape::Unit => {
                return None
            }
        };
        let (lo, hi) = (t.min(), t.max());
        Some(Interval {
            lo: (lo.neg, lo.mag),
            hi: (hi.neg, hi.mag),
        })
    }

    fn expr(&mut self, e: ExprId) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.expr(e).span;
        self.at(span);
        Ok(match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => {
                let n = Num::new(false, *value);
                let t = machine(n, n).ok_or("a literal beyond 128 bits")?;
                V::Int(self.int(t, n), t)
            }
            ExprKind::Bool(b) => V::Bool(self.bool(*b)),
            ExprKind::Name(_) => {
                let id = self.s.f.resolution.names[e.0 as usize].ok_or("a name never resolved")?;
                match self.s.locals[id.0 as usize].clone() {
                    Some(Local::Value(v)) => v,
                    Some(Local::Place(at, shape)) => self.load(at, &shape),
                    None => return Err("a name read before it is bound".into()),
                }
            }
            ExprKind::Todo(reason) => self.trap("todo", &format!("todo reached: {}", reason), span),
            ExprKind::Unreachable(reason) => self.trap(
                "unreachable",
                &format!("unreachable reached: {}", reason),
                span,
            ),
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Not => match self.expr(*operand)? {
                    V::Bool(b) => V::Bool(self.inst(Op::Not, Ty::Bool, &[b])),
                    V::Never => V::Never,
                    V::Int(..) | V::Mem(_) | V::Unit => {
                        return Err("`not` of a value that is not a bool".into())
                    }
                },
                UnaryOp::Neg => self.negate(e, *operand, span)?,
                UnaryOp::BitNot => {
                    let t = self.int_of(*operand).unwrap_or(I64);
                    let v = self.expr(*operand)?;
                    let Some((x, from)) = v.int_node("an operand of `~`")? else {
                        return Ok(V::Never);
                    };
                    let x = self.narrow_int(x, from, t, tree.expr(*operand).span, None);
                    V::Int(self.inst(Op::Not, Ty::Int(mir(t)), &[x]), mir(t))
                }
            },
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinaryOp::And | BinaryOp::Or => self.logic(*op, *lhs, *rhs)?,
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let l = self.expr(*lhs)?;
                    let r = self.expr(*rhs)?;
                    let op = match op {
                        BinaryOp::Eq => CmpOp::Eq,
                        BinaryOp::Ne => CmpOp::Ne,
                        BinaryOp::Lt => CmpOp::Lt,
                        BinaryOp::Le => CmpOp::Le,
                        BinaryOp::Gt => CmpOp::Gt,
                        BinaryOp::Ge => CmpOp::Ge,
                        // The arm holds comparisons only.
                        BinaryOp::Or
                        | BinaryOp::And
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::BitAnd
                        | BinaryOp::Shl
                        | BinaryOp::Shr
                        | BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Rem => return Err(format!("a comparison {:?}", op)),
                    };
                    match (l, r) {
                        (V::Int(a, ta), V::Int(b, tb)) => {
                            V::Bool(self.compare(op, (a, ta), (b, tb))?)
                        }
                        (V::Bool(a), V::Bool(b)) => {
                            V::Bool(self.inst(Op::Cmp(op), Ty::Bool, &[a, b]))
                        }
                        (V::Never, _) | (_, V::Never) => V::Never,
                        _ => return Err("a comparison of values that are not alike".into()),
                    }
                }
                BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitAnd
                | BinaryOp::Shl
                | BinaryOp::Shr
                | BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem => {
                    let l = self.expr(*lhs)?;
                    let r = self.expr(*rhs)?;
                    let ctx = self.interval(e);
                    let (li, ri) = (self.interval(*lhs), self.interval(*rhs));
                    self.binary(*op, (l, li, *lhs), (r, ri, *rhs), ctx, span)?
                }
            },
            ExprKind::Index { base, index } => {
                let b = self.expr(*base)?;
                // An array read from a place is its address; an index that changes the place
                // reads the array as it was, as a value is, so it is copied first.
                let b = self.kept(b, *base, &[*index])?;
                let i = self.expr(*index)?;
                // An index that never gives a value, such as `unreachable(...)`, ends the path.
                if i.never() {
                    return Ok(V::Never);
                }
                let Some(a) = b.mem_node("an indexed value")? else {
                    return Ok(V::Never);
                };
                let shape = self.shape(*base)?;
                let (len, elem) = shape.as_array("an indexed value")?;
                let at = self.element(a, len, elem, i, (span, tree.expr(*index).span))?;
                self.load(at, elem)
            }
            ExprKind::Field { base, .. } if no_value(&self.s.f.typing.exprs[base.0 as usize]) => {
                return self.leaves_first(&[*base]);
            }
            ExprKind::Field { base, name } => match self.shape(*base)? {
                Shape::Array(len, _) => {
                    self.expr(*base)?;
                    let n = Num::new(false, u128::from(len));
                    V::Int(self.int(USIZE, n), USIZE)
                }
                // A struct's value is its address, so its field is read where it lies.
                Shape::Struct(s) => {
                    let Some(a) = self.expr(*base)?.mem_node("a struct whose field is read")?
                    else {
                        return Ok(V::Never);
                    };
                    let (_, f) = s.field(name).ok_or("a field the struct lacks")?;
                    let at = self.field_addr(a, f.offset);
                    self.load(at, &f.shape)
                }
                Shape::Int(_) | Shape::Bool | Shape::Enum(_) | Shape::Unit => {
                    return Err("a field of a value that is neither an array nor a struct".into())
                }
            },
            // Each value is narrowed into its field's type and checked against its refinement,
            // in the order written, which is the declaration's, and stored where the layout
            // places its field.
            ExprKind::Construct { name, fields } => {
                let shape = self.shape(e)?;
                let s = shape.as_struct("a struct built of a type")?;
                let def = self
                    .m
                    .structs
                    .get(name)
                    .ok_or("a struct without a declaration")?;
                let slot = self.inst(Op::Slot { bytes: s.size }, Ty::Addr, &[]);
                for fi in fields {
                    let (k, fs) = s.field(&fi.name).ok_or("a field the struct lacks")?;
                    let decl = def.fields.get(k).ok_or("a field the struct lacks")?.decl;
                    let v = self.expr(fi.value)?;
                    let from = self.shape(fi.value)?;
                    let at = tree.expr(fi.value).span;
                    let v = self.land(v, &from, &fs.shape, Some((def.tree(), decl)), at)?;
                    if v.never() {
                        return Ok(V::Never);
                    }
                    let to = self.field_addr(slot, fs.offset);
                    self.store_shaped(to, v, &fs.shape);
                }
                V::Mem(slot)
            }
            // A variant: its tag first, then each value of its payload narrowed into its field's
            // type and checked against its refinement, in the order written, which is the
            // declaration's, and stored where the layout places its field.
            ExprKind::Variant {
                enum_name,
                name,
                payload,
                ..
            } => {
                let shape = self.shape(e)?;
                let s = shape.as_enum("a variant of a type")?;
                let def = self
                    .m
                    .structs
                    .get_enum(enum_name)
                    .ok_or("an enum without a declaration")?;
                let (tag, vs) = s.variant(name).ok_or("a variant the enum lacks")?;
                let (_, vdef) = def.variant(name).ok_or("a variant the enum lacks")?;
                let slot = self.inst(Op::Slot { bytes: s.size }, Ty::Addr, &[]);
                let t = mir(s.tag);
                let n = self.int(t, Num::new(false, tag as u128));
                self.inst(Op::Store, Ty::Unit, &[slot, n]);
                let fields = match payload {
                    Payload::Named(fields) => fields,
                    Payload::None => return Ok(V::Mem(slot)),
                    // A variant built by position is `E0226`.
                    Payload::Positional(_) => return Err("a variant built by position".into()),
                };
                for fi in fields {
                    let (k, fs) = vs.field(&fi.name).ok_or("a field the payload lacks")?;
                    let decl = vdef.fields.get(k).ok_or("a field the payload lacks")?.decl;
                    let v = self.expr(fi.value)?;
                    let from = self.shape(fi.value)?;
                    let at = tree.expr(fi.value).span;
                    let v = self.land(v, &from, &fs.shape, Some((def.tree(), decl)), at)?;
                    if v.never() {
                        return Ok(V::Never);
                    }
                    let to = self.field_addr(slot, fs.offset);
                    self.store_shaped(to, v, &fs.shape);
                }
                V::Mem(slot)
            }
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, *scrutinee, arms)?,
            ExprKind::Call { args, .. } => {
                let index =
                    self.s.f.resolution.calls[e.0 as usize].ok_or("a call never resolved")?;
                let callee = self.sig_of(index)?;
                let mut values = Vec::new();
                for (k, (a, (_, shape))) in args.iter().zip(&callee.params).enumerate() {
                    let aspan = tree.expr(a.value).span;
                    values.push(if a.is_mut {
                        let (root, steps) = self.place_path(a.value)?;
                        if reaches_never(&steps) {
                            return Ok(V::Never);
                        }
                        let at = self.address(root, &steps)?;
                        Arg::Place(at, shape.clone(), self.place_type(root, &steps)?, aspan)
                    } else {
                        let v = self.expr(a.value)?;
                        if v.never() {
                            return Ok(V::Never);
                        }
                        // A value read from a place, which a later argument may change, is
                        // passed as it was when it was evaluated.
                        let later: Vec<ExprId> = args[k + 1..].iter().map(|a| a.value).collect();
                        let v = self.kept(v, a.value, &later)?;
                        Arg::Value(v, aspan, self.shape(a.value)?)
                    });
                }
                let sret = callee.result.in_memory().then(|| {
                    self.inst(
                        Op::Slot {
                            bytes: callee.result.size(),
                        },
                        Ty::Addr,
                        &[],
                    )
                });
                let recursive = index == self.s.index;
                self.call(index, values, span, sret, recursive)?
            }
            ExprKind::Array(items) => {
                if no_value(&self.s.f.typing.exprs[e.0 as usize]) {
                    return self.leaves_first(items);
                }
                let shape = self.shape(e)?;
                let exact = self.exact(e);
                self.array_literal(items, &shape, exact, span)?
            }
            ExprKind::Repeat { value, .. } => {
                if no_value(&self.s.f.typing.exprs[e.0 as usize]) {
                    return self.leaves_first(&[*value]);
                }
                let shape = self.shape(e)?;
                let exact = self.exact(e);
                self.repeat_literal(*value, &shape, exact, span)?
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_expr(e, *cond, *then, *otherwise)?,
            ExprKind::Error => return Err("an expression the parser could not read".into()),
        })
    }

    /// The value `v` of `e`, kept as it is now while `after` is evaluated: an array or struct
    /// read from a place is that place's memory, which `after` may change through a `mut`
    /// argument or an assignment, so it is then copied first. Any other value is its own.
    /// Evaluates `items`, each of no value, in order, until one leaves, as each of them does.
    fn leaves_first(&mut self, items: &[ExprId]) -> Result<V, String> {
        for &x in items {
            if self.expr(x)?.never() {
                return Ok(V::Never);
            }
        }
        Err("a value of no type that gave one".into())
    }

    fn kept(&mut self, v: V, e: ExprId, after: &[ExprId]) -> Result<V, String> {
        let a = match v {
            V::Mem(a) => a,
            V::Int(..) | V::Bool(_) | V::Unit | V::Never => return Ok(v),
        };
        let mut roots = Vec::new();
        self.aliases(e, &mut roots);
        if roots.is_empty() || !after.iter().any(|&x| self.may_change_expr(x, &roots)) {
            return Ok(v);
        }
        let shape = self.shape(e)?;
        Ok(V::Mem(self.copy(a, &shape)))
    }

    /// The bindings whose memory the value of `e` may be: the root of a place, or of the places
    /// an `if` gives.
    fn aliases(&self, e: ExprId, out: &mut Vec<usize>) {
        let tree = self.tree();
        match &tree.expr(e).kind {
            ExprKind::Name(_) => {
                out.extend(self.s.f.resolution.names[e.0 as usize].map(|b| b.0 as usize))
            }
            ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => self.aliases(*base, out),
            ExprKind::If {
                then, otherwise, ..
            } => {
                let tail =
                    |b: AstBlock| match tree.block(b).stmts.last().map(|s| &tree.stmt(*s).kind) {
                        Some(StmtKind::Expr(x)) => Some(*x),
                        _ => None,
                    };
                if let Some(x) = tail(*then) {
                    self.aliases(x, out);
                }
                match otherwise {
                    Some(Else::Block(b)) => {
                        if let Some(x) = tail(*b) {
                            self.aliases(x, out);
                        }
                    }
                    Some(Else::If(i)) => self.aliases(*i, out),
                    None => {}
                }
            }
            // A `match` gives an arm's value, which may be a place; a pattern's binding is a
            // place of its scrutinee, which every arm that binds keeps as it is.
            ExprKind::Match { scrutinee, arms } => {
                self.aliases(*scrutinee, out);
                for arm in arms {
                    let x = match arm.body {
                        ArmBody::Expr(x) => Some(x),
                        ArmBody::Block(b) => {
                            match tree.block(b).stmts.last().map(|s| &tree.stmt(*s).kind) {
                                Some(StmtKind::Expr(x)) => Some(*x),
                                _ => None,
                            }
                        }
                    };
                    if let Some(x) = x {
                        self.aliases(x, out);
                    }
                }
            }
            // Every other value is a new one, in memory of its own or none.
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Error => {}
        }
    }

    /// Whether evaluating `e` may change a binding of `roots`: by a `mut` argument or an
    /// assignment, anywhere in it, whose place has one of them at its root.
    fn may_change_expr(&self, e: ExprId, roots: &[usize]) -> bool {
        let tree = self.tree();
        let sub = |x: ExprId| self.may_change_expr(x, roots);
        match &tree.expr(e).kind {
            ExprKind::Call { args, .. } => args.iter().any(|a| {
                (a.is_mut && self.root_of(a.value).is_some_and(|r| roots.contains(&r)))
                    || sub(a.value)
            }),
            ExprKind::Construct { fields, .. } => fields.iter().any(|f| sub(f.value)),
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => false,
                Payload::Named(fields) => fields.iter().any(|f| sub(f.value)),
                // A payload given by position is `E0226`, never lowered; read as one given by
                // name, its values.
                Payload::Positional(args) => args.iter().any(|a| sub(a.value)),
            },
            ExprKind::Match { scrutinee, arms } => {
                sub(*scrutinee)
                    || arms.iter().any(|a| match a.body {
                        ArmBody::Expr(x) => sub(x),
                        ArmBody::Block(b) => self.may_change_block(b, roots),
                    })
            }
            ExprKind::Unary { operand, .. } => sub(*operand),
            ExprKind::Binary { lhs, rhs, .. } => sub(*lhs) || sub(*rhs),
            ExprKind::Index { base, index } => sub(*base) || sub(*index),
            ExprKind::Field { base, .. } => sub(*base),
            ExprKind::Array(items) => items.iter().any(|&i| sub(i)),
            ExprKind::Repeat { value, .. } => sub(*value),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                sub(*cond)
                    || self.may_change_block(*then, roots)
                    || match otherwise {
                        Some(Else::Block(b)) => self.may_change_block(*b, roots),
                        Some(Else::If(i)) => sub(*i),
                        None => false,
                    }
            }
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => false,
        }
    }

    fn may_change_block(&self, b: AstBlock, roots: &[usize]) -> bool {
        let tree = self.tree();
        let sub = |x: ExprId| self.may_change_expr(x, roots);
        tree.block(b)
            .stmts
            .iter()
            .any(|&s| match &tree.stmt(s).kind {
                StmtKind::Assign { place, value, .. } => {
                    self.root_of(*place).is_some_and(|r| roots.contains(&r))
                        || sub(*place)
                        || sub(*value)
                }
                StmtKind::Binding { init: x, .. }
                | StmtKind::Discard(x)
                | StmtKind::Expr(x)
                | StmtKind::Return(Some(x)) => sub(*x),
                StmtKind::While {
                    cond,
                    invariants,
                    decreases,
                    body,
                } => {
                    sub(*cond)
                        || invariants.iter().chain(decreases).any(|&x| sub(x))
                        || self.may_change_block(*body, roots)
                }
                StmtKind::Guard { cond, otherwise } => {
                    sub(*cond) || self.may_change_block(*otherwise, roots)
                }
                StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => false,
            })
    }

    /// The binding at the root of a place, none for a value that is not one.
    fn root_of(&self, e: ExprId) -> Option<usize> {
        match &self.tree().expr(e).kind {
            ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => self.root_of(*base),
            ExprKind::Name(_) => self.s.f.resolution.names[e.0 as usize].map(|b| b.0 as usize),
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => None,
        }
    }

    /// The integer type of an expression, as the interpreter's operations take it.
    fn int_of(&self, e: ExprId) -> Option<SemaIntTy> {
        match &self.s.f.typing.exprs[e.0 as usize] {
            SemaTy::Int(i) => int_ty(i),
            SemaTy::Bool
            | SemaTy::Array(..)
            | SemaTy::Struct(_)
            | SemaTy::Enum(_)
            | SemaTy::Unit
            | SemaTy::Any => None,
        }
    }

    fn logic(&mut self, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> Result<V, String> {
        let Some(l) = self.expr(lhs)?.bool_node("an operand of `and` or `or`")? else {
            return Ok(V::Never);
        };
        let right = self.block(&[]);
        let join = self.block(&[Ty::Bool]);
        // `and` stops at false, `or` at true, with that value.
        let (then, otherwise) = if op == BinaryOp::And {
            ((right, vec![]), (join, vec![l]))
        } else {
            ((join, vec![l]), (right, vec![]))
        };
        self.end(Term::Branch {
            cond: l,
            then,
            otherwise,
        });
        self.b.switch(right);
        if let Some(r) = self.expr(rhs)?.bool_node("an operand of `and` or `or`")? {
            self.jump(join, vec![r]);
        }
        self.b.switch(join);
        Ok(V::Bool(self.b.params(join)[0]))
    }

    fn if_expr(
        &mut self,
        e: ExprId,
        cond: ExprId,
        then: AstBlock,
        otherwise: Option<Else>,
    ) -> Result<V, String> {
        let Some(c) = self.expr(cond)?.bool_node("a condition")? else {
            return Ok(V::Never);
        };
        let shape = self.shape(e)?;
        // An integer value meets in one machine type, from the `if`'s static range.
        let join_ty = match (&shape, self.interval(e)) {
            (Shape::Int(_), Some(iv)) => Some(Ty::Int(
                machine(Num::new(iv.lo.0, iv.lo.1), Num::new(iv.hi.0, iv.hi.1))
                    .ok_or("an `if` beyond 128 bits")?,
            )),
            (Shape::Int(t), None) => Some(Ty::Int(mir(*t))),
            (Shape::Bool | Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_), _) => {
                Some(shape.ty())
            }
            _ => None,
        };
        let yes = self.block(&[]);
        let no = self.block(&[]);
        let join = self.block(join_ty.as_slice());
        self.end(Term::Branch {
            cond: c,
            then: (yes, vec![]),
            otherwise: (no, vec![]),
        });
        self.b.switch(yes);
        let v = self.body(then)?;
        self.meet(v, join, join_ty);
        self.b.switch(no);
        let v = match otherwise {
            Some(Else::Block(b)) => self.body(b)?,
            Some(Else::If(i)) => self.expr(i)?,
            None => V::Unit,
        };
        self.meet(v, join, join_ty);
        self.b.switch(join);
        // An `if` of no type has branches that all leave, so nothing reaches its end.
        if no_value(&self.s.f.typing.exprs[e.0 as usize]) {
            return Ok(V::Never);
        }
        self.joined(join, join_ty)
    }

    /// A `match` as branches (`core-calculus.md`, item 14): its scrutinee is evaluated once,
    /// then each arm's pattern is tested in order, an enum's by its tag, and the first that
    /// matches runs. The last arm is not tested, since the `match` is exhaustive. A pattern's
    /// bindings are its payload's fields where they lie, which the arm only reads.
    fn match_expr(&mut self, e: ExprId, scrutinee: ExprId, arms: &[Arm]) -> Result<V, String> {
        let s = self.expr(scrutinee)?;
        let read = self.shape(scrutinee)?;
        // The checker freezes a scrutinee that is a place in every arm that binds a part of
        // it. One that is no place, such as an `if` that gives one of two places, a field or
        // an element of one, is a value: an arm that changes the place it was read from must
        // not reach the arm's bindings, so it is copied first when an arm may.
        let s = match s {
            V::Mem(a) if !self.is_place(scrutinee) => {
                let mut roots = Vec::new();
                self.aliases(scrutinee, &mut roots);
                let changed = arms.iter().any(|arm| match arm.body {
                    ArmBody::Expr(x) => self.may_change_expr(x, &roots),
                    ArmBody::Block(b) => self.may_change_block(b, &roots),
                });
                if !roots.is_empty() && changed {
                    V::Mem(self.copy(a, &read))
                } else {
                    s
                }
            }
            // An integer scrutinee is narrowed into its type, which the arms cover, unless
            // Tier 1 proves it fits ([`match-scrutinee-narrowing`]); literals alone are an
            // `i64`. So no value the arms leave out reaches the last arm, which has no test.
            V::Int(x, t) => {
                let to = match self.int_of(scrutinee) {
                    Some(to) => to,
                    None => SemaIntTy::named("i64").ok_or("no `i64`")?,
                };
                let at = self.tree().expr(scrutinee).span;
                V::Int(self.narrow_int(x, t, to, at, Some(at)), mir(to))
            }
            s @ (V::Bool(_) | V::Mem(_) | V::Unit | V::Never) => s,
        };
        // What the patterns test: an enum's tag, a `bool`, or an integer.
        let tested = match (s, &read) {
            (V::Never, _) => return Ok(V::Never),
            (V::Mem(a), Shape::Enum(es)) => {
                let t = mir(es.tag);
                V::Int(self.inst(Op::Load, Ty::Int(t), &[a]), t)
            }
            (v, _) => v,
        };
        let shape = self.shape(e)?;
        let join_ty = match (&shape, self.interval(e)) {
            (Shape::Int(_), Some(iv)) => Some(Ty::Int(
                machine(Num::new(iv.lo.0, iv.lo.1), Num::new(iv.hi.0, iv.hi.1))
                    .ok_or("a `match` beyond 128 bits")?,
            )),
            (Shape::Int(t), None) => Some(Ty::Int(mir(*t))),
            (Shape::Bool | Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_), _) => {
                Some(shape.ty())
            }
            _ => None,
        };
        let join = self.block(join_ty.as_slice());
        for (k, arm) in arms.iter().enumerate() {
            let last = k + 1 == arms.len();
            let test = match (&arm.pattern.kind, tested) {
                // `_` is the last arm, since an arm after it is `E0230`.
                (_, _) if last => None,
                (PatternKind::Bool(b), V::Bool(c)) => Some(if *b {
                    c
                } else {
                    self.inst(Op::Not, Ty::Bool, &[c])
                }),
                (
                    PatternKind::Int {
                        negative, value, ..
                    },
                    V::Int(x, t),
                ) => {
                    let n = Num::new(*negative, *value);
                    let lt = machine(n, n).ok_or("a pattern beyond 128 bits")?;
                    let lit = self.int(lt, n);
                    Some(self.compare(CmpOp::Eq, (x, t), (lit, lt))?)
                }
                (PatternKind::Variant { name, .. }, V::Int(tag, t)) => {
                    let es = read.as_enum("a variant pattern on a value")?;
                    let (k, _) = es.variant(name).ok_or("a variant the enum lacks")?;
                    let n = self.int(t, Num::new(false, k as u128));
                    Some(self.inst(Op::Cmp(CmpOp::Eq), Ty::Bool, &[tag, n]))
                }
                _ => return Err("a pattern of another kind than its scrutinee".into()),
            };
            let next = match test {
                Some(c) => {
                    let run = self.block(&[]);
                    let next = self.block(&[]);
                    self.end(Term::Branch {
                        cond: c,
                        then: (run, vec![]),
                        otherwise: (next, vec![]),
                    });
                    self.b.switch(run);
                    Some(next)
                }
                None => None,
            };
            // The bindings read the payload where it lies in the scrutinee's memory.
            if let (PatternKind::Variant { name, bindings, .. }, V::Mem(a), Shape::Enum(es)) =
                (&arm.pattern.kind, s, &read)
            {
                let (_, vs) = es.variant(name).ok_or("a variant the enum lacks")?;
                for (i, b) in bindings.iter().flatten().enumerate() {
                    let Some(id) = self.s.f.resolution.patterns.get(&(e, k, i)) else {
                        continue;
                    };
                    let f = match &b.field {
                        Some((n, _)) => vs.field(n).map(|(_, f)| f),
                        None => vs.fields.get(i),
                    }
                    .ok_or("a binding of a field the payload lacks")?;
                    let at = self.field_addr(a, f.offset);
                    self.s.locals[id.0 as usize] = Some(Local::Place(at, f.shape.clone()));
                }
            }
            let v = match arm.body {
                ArmBody::Expr(x) => self.expr(x)?,
                ArmBody::Block(b) => self.body(b)?,
            };
            self.meet(v, join, join_ty);
            match next {
                Some(next) => self.b.switch(next),
                None => break,
            }
        }
        self.b.switch(join);
        // A `match` of no type has arms that all leave, so nothing reaches its end.
        if no_value(&self.s.f.typing.exprs[e.0 as usize]) {
            return Ok(V::Never);
        }
        self.joined(join, join_ty)
    }

    /// Whether `e` is a place: a name, followed by fields and indexes.
    fn is_place(&self, e: ExprId) -> bool {
        match &self.tree().expr(e).kind {
            ExprKind::Name(_) => true,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.is_place(*base),
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Error => false,
        }
    }

    /// The value an `if` or a `match` gives at `join`, the block its branches meet in: its
    /// parameter, of the type they meet in, or `()` when they give none.
    fn joined(&mut self, join: BlockId, ty: Option<Ty>) -> Result<V, String> {
        let Some(ty) = ty else {
            return Ok(V::Unit);
        };
        let p = || {
            self.b
                .params(join)
                .first()
                .copied()
                .ok_or("a value without its parameter where its branches meet")
        };
        Ok(match ty {
            Ty::Int(t) => V::Int(p()?, t),
            Ty::Bool => V::Bool(p()?),
            Ty::Addr => V::Mem(p()?),
            Ty::Unit => V::Unit,
        })
    }

    /// Jumps from a branch of an `if` to `join` with its value.
    fn meet(&mut self, v: V, join: BlockId, ty: Option<Ty>) {
        let args = match (v, ty) {
            (V::Never, _) => return,
            (V::Int(n, t), Some(Ty::Int(to))) => vec![self.cast(n, t, to)],
            (V::Bool(n) | V::Mem(n), Some(_)) => vec![n],
            _ => vec![],
        };
        self.jump(join, args);
    }

    fn negate(&mut self, e: ExprId, operand: ExprId, span: Span) -> Result<V, String> {
        // A literal's negation is a constant. `-170141183460469231731687303715884105728` is
        // `i128`'s smallest value, though its literal lies outside `i128` (`core-calculus.md`
        // item 5) and no operation could hold both; and on a literal of any type but `u128`,
        // whose negation the operator checks, it never traps.
        if let ExprKind::Int { value, .. } = self.tree().expr(operand).kind {
            if self
                .int_of(operand)
                .is_none_or(|w| w.signed || w.bits < 128)
            {
                let n = Num::new(value != 0, value);
                let m = machine(n, n).ok_or("a literal beyond 128 bits")?;
                return Ok(V::Int(self.int(m, n), m));
            }
        }
        let v = self.expr(operand)?;
        let Some((x, t)) = v.int_node("an operand of `-`")? else {
            return Ok(V::Never);
        };
        if let Some(w) = self.int_of(operand).filter(|w| w.bits == 128) {
            // On an i128 or u128, checked at the operator.
            let w = mir(w);
            let x = self.cast(x, t, w);
            let after = format!(
                " does not fit in `{}`",
                self.int_of(operand).map_or("", |t| t.name)
            );
            self.at(span);
            let site = self.b.trap_site("narrowing", &["", &after]);
            return Ok(V::Int(
                self.inst(Op::Neg { trap: Some(site) }, Ty::Int(w), &[x]),
                w,
            ));
        }
        let m = self.op_type(&[self.interval(operand), self.interval(e)], t)?;
        let x = self.cast(x, t, m);
        Ok(V::Int(
            self.inst(Op::Neg { trap: None }, Ty::Int(m), &[x]),
            m,
        ))
    }

    /// The machine type of an exact operation: the narrowest that holds its operands' and its
    /// result's ranges.
    fn op_type(&self, ranges: &[Option<Interval>], fallback: IntTy) -> Result<IntTy, String> {
        let mut lo: Option<Num> = None;
        let mut hi: Option<Num> = None;
        for r in ranges {
            let Some(r) = r else {
                return Ok(fallback);
            };
            let (l, h) = (Num::new(r.lo.0, r.lo.1), Num::new(r.hi.0, r.hi.1));
            lo = Some(lo.map_or(l, |x| x.min(l)));
            hi = Some(hi.map_or(h, |x| x.max(h)));
        }
        match (lo, hi) {
            (Some(lo), Some(hi)) => {
                machine(lo, hi).ok_or_else(|| "an operation beyond 128 bits".into())
            }
            _ => Ok(fallback),
        }
    }

    /// An arithmetic, bit or shift operation on two lowered operands, each with its range and
    /// expression; `ctx` is the operation's own static type and range.
    fn binary(
        &mut self,
        op: BinaryOp,
        (l, li, le): (V, Option<Interval>, ExprId),
        (r, ri, re): (V, Option<Interval>, ExprId),
        ctx: Option<Interval>,
        span: Span,
    ) -> Result<V, String> {
        let what = format!("an operand of `{}`", op.symbol());
        let Some((a, ta)) = l.int_node(&what)? else {
            return Ok(V::Never);
        };
        let Some((b, tb)) = r.int_node(&what)? else {
            return Ok(V::Never);
        };
        let tree = self.tree();
        let lt = self.int_of(le);
        let rt = self.int_of(re);
        self.at(span);
        match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let t = lt.unwrap_or(I64);
                let w = mir(t);
                // The amount lies within the type's width, unless Tier 1 proves it does.
                if !self.proven(
                    ObligationKind::Shift,
                    Key::at(tree.expr(re).span, Tag::None),
                ) {
                    let zero = self.int(tb, Num::ZERO);
                    let ge = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[b, zero]);
                    let width = Num::new(false, u128::from(t.bits));
                    let below = if tb.holds(width) {
                        let n = self.int(tb, width);
                        self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[b, n])
                    } else {
                        self.bool(true)
                    };
                    let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
                    let after = format!(" is outside 0 ..= {}", t.bits - 1);
                    self.check(ok, "shift-amount", &["shift amount ", &after], &[b], span);
                }
                let k = self.cast(b, tb, USIZE);
                if op == BinaryOp::Shl {
                    let after = format!(" does not fit in `{}`", t.name);
                    let site = self.b.trap_site("narrowing", &["", &after]);
                    Ok(V::Int(
                        self.inst(Op::Shl { trap: site }, Ty::Int(w), &[a, k]),
                        w,
                    ))
                } else {
                    let s = self.inst(Op::Shr, Ty::Int(ta), &[a, k]);
                    Ok(V::Int(self.narrow_int(s, ta, t, span, Some(span)), w))
                }
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let t = lt.or(rt).unwrap_or(I64);
                let (ls, rs) = (tree.expr(le).span, tree.expr(re).span);
                let x = self.narrow_int(a, ta, t, ls, Some(ls));
                let y = self.narrow_int(b, tb, t, rs, Some(rs));
                let bit = match op {
                    BinaryOp::BitAnd => BitOp::And,
                    BinaryOp::BitOr => BitOp::Or,
                    BinaryOp::BitXor => BitOp::Xor,
                    // The arm holds bit operations only.
                    BinaryOp::Or
                    | BinaryOp::And
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Shl
                    | BinaryOp::Shr
                    | BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => return Err(format!("a bit operation {:?}", op)),
                };
                self.at(span);
                Ok(V::Int(
                    self.inst(Op::Bits(bit), Ty::Int(mir(t)), &[x, y]),
                    mir(t),
                ))
            }
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Rem => {
                let arith = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    BinaryOp::Div => ArithOp::Div,
                    BinaryOp::Rem => ArithOp::Rem,
                    BinaryOp::Or
                    | BinaryOp::And
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitAnd
                    | BinaryOp::Shl
                    | BinaryOp::Shr => return Err(format!("an operation {:?} on integers", op)),
                };
                let wide = [lt, rt].into_iter().flatten().find(|t| t.bits == 128);
                let m = match wide {
                    Some(w) => mir(w),
                    None => self.op_type(&[li, ri, ctx], ta)?,
                };
                // An operand whose type the operation's does not hold, an `i32` beside a `u128`,
                // keeps its own: only a checked operation, which computes its exact result
                // first, takes operands of other types.
                let keeps =
                    |t: IntTy| wide.is_some() && !(m.min() <= t.min() && t.max() <= m.max());
                let mixed = keeps(ta) || keeps(tb);
                let x = if keeps(ta) { a } else { self.cast(a, ta, m) };
                let (y, ty) = if keeps(tb) {
                    (b, tb)
                } else {
                    (self.cast(b, tb, m), m)
                };
                let divides = matches!(arith, ArithOp::Div | ArithOp::Rem);
                let divisor = Key::at(tree.expr(re).span, Tag::None);
                if divides && !self.proven(ObligationKind::Divisor, divisor) {
                    let zero = self.int(ty, Num::ZERO);
                    let ok = self.inst(Op::Cmp(CmpOp::Ne), Ty::Bool, &[y, zero]);
                    self.check(ok, "division-by-zero", &["division by zero"], &[], span);
                }
                // An operation on a 128-bit operand is checked unless Tier 1 proves its result
                // fits, and always when its operands have other types.
                let trap = wide
                    .filter(|w| {
                        mixed
                            || !self.proven(ObligationKind::Range, Key::at(span, Tag::Into(w.name)))
                    })
                    .map(|w| {
                        let after = format!(" does not fit in `{}`", w.name);
                        self.b.trap_site("narrowing", &["", &after])
                    });
                Ok(V::Int(
                    self.inst(Op::Arith { op: arith, trap }, Ty::Int(m), &[x, y]),
                    m,
                ))
            }
        }
    }

    // -- calls -----------------------------------------------------------------------------

    /// Calls function `index` with `args`, as the reference interpreter does: each argument
    /// narrowed into its parameter's type at its own span, the parameters' refinements checked,
    /// then the callee's `requires` and its `decreases` measure, at `at`; for a `recursive`
    /// call, the measure must be smaller than this function's. After the call, a `mut` place
    /// is checked against its declared refinement.
    fn call(
        &mut self,
        index: usize,
        args: Vec<Arg<'a>>,
        at: Span,
        sret: Option<NodeId>,
        recursive: bool,
    ) -> Result<V, String> {
        let sig = self.sig_of(index)?;
        let mut callee = Scope::new(self.m, index)?;
        // Each argument narrowed into its parameter's type, at its own span.
        let mut values = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let shape = &sig.params[i].1;
            values.push(match arg {
                Arg::Value(v, span, from) => {
                    let v = self.narrow(*v, from, shape, *span, Some(*span))?;
                    if v.never() {
                        return Ok(V::Never);
                    }
                    callee.locals[i] = Some(match v {
                        V::Mem(a) => Local::Place(a, shape.clone()),
                        v @ (V::Int(..) | V::Bool(_) | V::Unit | V::Never) => Local::Value(v),
                    });
                    match v {
                        V::Int(n, _) | V::Bool(n) | V::Mem(n) => n,
                        V::Unit | V::Never => return Err("an argument without a value".into()),
                    }
                }
                Arg::Place(a, shape, _, _) => {
                    callee.locals[i] = Some(Local::Place(*a, shape.clone()));
                    *a
                }
            });
        }
        // Then each parameter's refinement, in the callee's scope, since a bound may name
        // another parameter.
        let caller = self.measure.clone();
        let saved = std::mem::replace(&mut self.s, callee);
        let was = std::mem::replace(&mut self.in_callee, true);
        let outcome = self.callee_checks(&args, at, recursive, caller);
        self.in_callee = was;
        self.s = saved;
        outcome?;
        let mut mir_args = Vec::new();
        mir_args.extend(sret);
        mir_args.extend(values);
        let (_, result_ty) = mir_params(sig);
        self.at(at);
        let r = self.inst(Op::Call(sig.func), result_ty.unwrap_or(Ty::Unit), &mir_args);
        // A `mut` place takes back what the callee left, which enters its refinement. A struct's
        // fields hold theirs in any value the callee leaves, so they are not checked again.
        let entry = std::mem::replace(&mut self.entry, false);
        for arg in &args {
            if let Arg::Place(a, shape, Some(d), arg_span) = arg {
                let v = self.load(*a, shape);
                let back = self.refined(v, shape, *d, at, Key::at(*arg_span, Tag::Back));
                if back.is_err() {
                    self.entry = entry;
                }
                back?;
            }
        }
        self.entry = entry;
        Ok(match (&sig.result, sret) {
            (Shape::Int(t), _) => V::Int(r, mir(*t)),
            (Shape::Bool, _) => V::Bool(r),
            // Only a result kept in memory has a slot.
            (_, Some(slot)) => V::Mem(slot),
            (Shape::Unit, None) => V::Unit,
            (Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_), None) => {
                return Err("a result in memory without its slot".into())
            }
        })
    }

    /// The checks a call makes in the callee's scope: the parameters' refinements, `requires`
    /// and the `decreases` measure, against `caller`'s for a recursive call.
    fn callee_checks(
        &mut self,
        args: &[Arg<'a>],
        at: Span,
        recursive: bool,
        caller: Option<Vec<V>>,
    ) -> Result<(), String> {
        let tree = self.tree();
        for (i, (p, arg)) in self.s.f.syntax.params.clone().iter().zip(args).enumerate() {
            let (Arg::Value(_, span, _) | Arg::Place(_, _, _, span)) = arg;
            let span = *span;
            if let Some(local) = self.s.locals[i].clone() {
                let shape = Shape::of(&SemaTy::of(tree, p.ty), self.shapes)?;
                let v = match local {
                    Local::Value(v) => v,
                    Local::Place(a, s) => self.load(a, &s),
                };
                self.refined(v, &shape, (tree, p.ty), span, Key::call(span, Tag::None))?;
            }
        }
        for c in self.clauses(ContractKind::Requires) {
            let text = format!("`requires {}` is false at this call", tree.render_expr(c));
            let key = Key::call(at, Tag::Clause(tree.expr(c).span));
            self.clause(c, ("requires", ObligationKind::Requires), &text, at, key)?;
        }
        let clauses = self.clauses(ContractKind::Decreases);
        if let Some(&first) = clauses.first() {
            let now = self.measure(&clauses, Some(at))?;
            let proven = self.proven(ObligationKind::Decreases, Key::call(at, Tag::Smaller));
            if let (true, Some(before), false) = (recursive, caller, proven) {
                let smaller = self.lex_less(&now, &before)?;
                let text = format!(
                    "`decreases {}` is negative or did not decrease",
                    tree.render_expr(first)
                );
                self.check(smaller, "decreases", &[&text], &[], at);
            }
        }
        Ok(())
    }
}

/// The integer type in which two of `a` and `b` compare, when there is one: a u128 and a
/// signed type have none.
fn common(a: IntTy, b: IntTy) -> Option<IntTy> {
    if a == b {
        return Some(a);
    }
    match (a.signed, b.signed) {
        (false, false) | (true, true) => Some(IntTy::new(a.bits.max(b.bits), a.signed)),
        _ => {
            let (u, s) = if a.signed { (b, a) } else { (a, b) };
            if u.bits == 128 {
                return None;
            }
            Some(IntTy::new(s.bits.max(u.bits * 2), true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U16: SemaIntTy = SemaIntTy {
        name: "u16",
        signed: false,
        bits: 16,
    };

    #[test]
    fn test_shapes_have_the_size_and_type_of_their_values() {
        assert_eq!(Shape::Int(U16).size(), 2);
        assert_eq!(Shape::Bool.size(), 1);
        let grid = Shape::Array(3, Box::new(Shape::Array(2, Box::new(Shape::Int(U16)))));
        assert_eq!(grid.size(), 12);
        assert_eq!(Shape::Unit.size(), 0);
        assert_eq!(grid.ty(), Ty::Addr);
        assert_eq!(Shape::Int(U16).ty(), Ty::Int(IntTy::new(16, false)));
        assert_eq!(Shape::Bool.ty(), Ty::Bool);
        assert_eq!(
            Shape::of(
                &SemaTy::Array(2, Box::new(SemaTy::Bool)),
                &Shapes::default()
            ),
            Ok(Shape::Array(2, Box::new(Shape::Bool)))
        );
        assert!(Shape::of(&SemaTy::Any, &Shapes::default()).is_err());
    }

    /// An array literal of integers of no standard type together keeps its elements exact, in
    /// the machine type that holds them all, and narrows them only where it lands.
    #[test]
    fn test_an_array_literal_of_mixed_integers_keeps_its_elements_exact() {
        let src = "module m
pub fn m(a: u128, b: i8[0..10]) -> [2]u128 = [a, b]
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        // Kept in `u128`, which holds both: no narrowing at the literal, none where it lands.
        assert_eq!(checks(&lowered, "m"), Vec::<String>::new());
        let text = lowered.module.to_string();
        assert!(text.contains("slot 32"), "{}", text);
        // An array literal of array literals keeps its integers exact at every depth, in the
        // type that holds them all, here `u64`: nothing narrows them into `i64` on the way.
        let src = "module m
pub fn take(p: [2][1]usize) -> u8 = 0
pub fn arg() -> u8 = take([[0x0], [12960978670946228878]])
pub fn nested() -> [2][1]u64 = [[1], [12960978670946228878]]
pub fn repeated() -> [2][2]u64 = [[0, 12960978670946228878]; 2]
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        for f in ["arg", "nested", "repeated"] {
            assert_eq!(checks(&lowered, f), Vec::<String>::new(), "{}", f);
        }
    }

    /// An expression of no type gives no value: a `todo` body, and an `if` whose branches all
    /// return, lower, and nothing reaches their end but the trap that marks it unreachable.
    #[test]
    fn test_an_expression_of_no_type_lowers_to_no_value() {
        let source = "module m
pub fn f() -> i32 = todo(\"x\")
pub fn g(c: bool) -> i32 {
  if c {
    return 1
  } else {
    return 2
  }
}
";
        let module = mente_sema::check_program(source).unwrap();
        let lowered = lower(&module).unwrap();
        let text = lowered.module.to_string();
        assert!(text.contains("todo reached: x"), "{}", text);
        let g = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == "g")
            .unwrap();
        // Both branches return; the block after the `if` only traps.
        let returns = g
            .blocks
            .iter()
            .filter(|b| matches!(b.term, Term::Return(_)))
            .count();
        assert_eq!(returns, 2, "{}", g);
    }

    /// An array literal or a repeat whose elements have no type gives no value, and neither does
    /// a field, a length or an element read of a value of no type (`core-calculus.md`, item 6):
    /// each evaluates what it holds in order, which leaves at the first that does. A literal
    /// whose first element returns returns from there.
    #[test]
    fn test_an_aggregate_of_no_value_lowers_to_no_value() {
        let source = "module m
struct P {
  a: u8
  b: u16
}
pub fn single(c: bool) -> u8 {
  val a: [1]u8 = [if c { todo(\"a\") } else { todo(\"b\") }]
  a[0]
}
pub fn bare() -> u8 {
  val a = [todo(\"a\")]
  a[0]
}
pub fn nested(c: bool) -> u8 {
  val a = [[if c { todo(\"a\") } else { todo(\"b\") }]]
  a[0][0]
}
pub fn repeated(c: bool) -> u8 {
  val a: [3]u8 = [if c { todo(\"a\") } else { todo(\"b\") }; 3]
  a[0]
}
pub fn length(c: bool) -> usize = (if c { todo(\"a\") } else { todo(\"b\") }).len
pub fn bare_length() -> usize = todo(\"a\").len
pub fn field(c: bool) -> u8 = (if c { todo(\"a\") } else { todo(\"b\") }).a
pub fn indexed(c: bool) -> u8 = (if c { todo(\"a\") } else { todo(\"b\") })[0]
pub fn returns(c: bool) -> u8 {
  val a = [if c { return 7 } else { return 8 }, if c { todo(\"a\") } else { todo(\"b\") }]
  a[0]
}
";
        let module = mente_sema::check_program(source).unwrap();
        let lowered = lower(&module).unwrap();
        let returns = |f: &str| {
            let func = lowered
                .module
                .functions
                .iter()
                .find(|g| g.name == f)
                .unwrap();
            func.blocks
                .iter()
                .filter(|b| matches!(b.term, Term::Return(_)))
                .count()
        };
        for f in [
            "single",
            "bare",
            "nested",
            "repeated",
            "length",
            "bare_length",
            "field",
            "indexed",
        ] {
            assert_eq!(returns(f), 0, "{}: every path traps", f);
        }
        assert_eq!(returns("returns"), 2, "the first element returns 7 or 8");
    }

    /// An `if` or a `match` whose every branch leaves has no type and gives no value
    /// (`core-calculus.md`, item 6), wherever it stands: what would hold its value, a binding,
    /// an element of an array literal or an argument, is never reached, so it needs no shape.
    #[test]
    fn test_a_value_whose_every_branch_leaves_lowers_to_no_value() {
        let source = "module m
pub fn bound(c: bool, x: u8) -> u8 {
  val y = if c { todo(\"a\") } else { todo(\"b\") }
  y
}
pub fn matched(c: bool) -> u8 {
  val y = match c {
    true => todo(\"a\")
    false => unreachable(\"b\")
  }
  y
}
pub fn element(c: bool, x: u8) -> u8 {
  val a: [2]u8 = [if c { todo(\"a\") } else { todo(\"b\") }, 1]
  a[1]
}
pub fn argument(c: bool, x: u8) -> u8 = id(if c { todo(\"a\") } else { todo(\"b\") })
fn id(x: u8) -> u8 = x
";
        let module = mente_sema::check_program(source).unwrap();
        let lowered = lower(&module).unwrap();
        let text = lowered.module.to_string();
        for f in ["bound", "matched", "element", "argument"] {
            let func = lowered
                .module
                .functions
                .iter()
                .find(|g| g.name == f)
                .unwrap();
            // Nothing returns: every path traps.
            assert!(
                !func
                    .blocks
                    .iter()
                    .any(|b| matches!(b.term, Term::Return(_))),
                "{}",
                func
            );
        }
        assert!(text.contains("todo reached: a"), "{}", text);
    }

    /// A struct lies in one slot, its fields at the offsets the checker's layout gives them,
    /// largest alignment first: here `wide` at 0, `n` at 16, `flag` at 20; a field is read
    /// and stored there, and a struct is passed and returned by its address, as an array is.
    #[test]
    fn test_struct_fields_lie_where_the_layout_places_them() {
        let src = "module m
pub struct S {
  flag: bool
  n: u32
  wide: i128
}
pub fn make(n: u32) -> S = S(flag: true, n: n, wide: 7)
pub fn get(s: S) -> u32 = s.n
pub fn set(mut s: S) {
  s.flag = false
}
";
        let m = mente_sema::check_program(src).unwrap();
        let shapes = Shapes::of(&m.structs).unwrap();
        let Shape::Struct(st) = Shape::of(&SemaTy::Struct("S".into()), &shapes).unwrap() else {
            panic!("a struct's shape");
        };
        let offsets: Vec<(String, u32)> = st
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.offset))
            .collect();
        assert_eq!(
            offsets,
            [("flag", 20), ("n", 16), ("wide", 0)].map(|(n, o)| (n.to_string(), o))
        );
        assert_eq!((st.size, st.align), (32, 16));
        let lowered = lower(&m).unwrap();
        let text = lowered.module.to_string();
        let f = |name: &str| {
            lowered
                .module
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .to_string()
        };
        // `make` returns through the address its caller passes, and builds in a slot of 32.
        assert!(f("make").contains("fn make(addr, u32)"), "{}", text);
        assert!(f("make").contains("slot 32"), "{}", text);
        // A field away from the start is reached at its offset.
        assert!(f("get").contains("elem 1"), "{}", f("get"));
        assert!(f("get").contains("= int 16"), "{}", f("get"));
        assert!(f("set").contains("= int 20"), "{}", f("set"));
    }

    /// An enum lies tag first in its slot, the payloads after it where its layout places them; a
    /// variant is built by storing its tag and its payload's fields, and a `match` loads the tag
    /// and branches on it, its last arm untested.
    #[test]
    fn test_an_enum_lies_tag_first_in_its_slot() {
        let src = "module m
pub enum E {
  A
  B(flag: bool, wide: u64)
}
pub fn make(w: u64) -> E = E.B(flag: true, wide: w)
pub fn get(e: E) -> u64 = match e {
  E.B(flag: _, wide: w) => w
  E.A => 0
}
";
        let m = mente_sema::check_program(src).unwrap();
        let shapes = Shapes::of(&m.structs).unwrap();
        let Shape::Enum(en) = Shape::of(&SemaTy::Enum("E".into()), &shapes).unwrap() else {
            panic!("an enum's shape");
        };
        assert_eq!((en.tag.name, en.size, en.align), ("u8", 24, 8));
        let offsets: Vec<u32> = en.variants[1].fields.iter().map(|f| f.offset).collect();
        assert_eq!(offsets, [16, 8]);
        let lowered = lower(&m).unwrap();
        let f = |name: &str| {
            lowered
                .module
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .to_string()
        };
        // `make` builds in a slot of 24 and stores tag 1 first.
        assert!(f("make").contains("slot 24"), "{}", f("make"));
        assert!(f("make").contains("= int 1"), "{}", f("make"));
        // `get` loads the tag, compares it with 1, and reads `wide` at its offset.
        assert!(f("get").contains(": u8 = load v0"), "{}", f("get"));
        assert!(f("get").contains("= int 8"), "{}", f("get"));
        assert!(f("get").contains("bool = eq v1"), "{}", f("get"));
    }

    /// An array or a struct read from a binding is passed by its address, and copied first only
    /// where a later operand may change the binding: so the value an argument has is the one it
    /// had when it was evaluated, as the reference interpreter has it.
    #[test]
    fn test_an_argument_keeps_its_value_though_a_later_operand_changes_it() {
        let src = "module m
fn first(xs: [2]u8, k: u8) -> u8 = xs[0] + k
fn bump(mut xs: [2]u8) -> u8 {
  xs[0] = 7
  0
}
pub fn changed() -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, bump(mut xs))
}
pub fn kept() -> u8 {
  var xs: [2]u8 = [1, 2]
  xs[1] = 3
  first(xs, 0)
}
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let copies = |name: &str| {
            let f = lowered
                .module
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap();
            f.ops
                .iter()
                .filter(|op| matches!(op, Op::Copy { .. }))
                .count()
        };
        // The literal's copy into the binding, then the argument's copy before `bump` runs.
        assert_eq!(copies("changed"), 2);
        assert_eq!(copies("kept"), 1);
    }

    /// An argument read from a binding is copied exactly where a later operand may change that
    /// binding, by a `mut` argument or an assignment, wherever the operand holds it: in an
    /// operator, an index, an `if`'s blocks, a loop's condition or body, a guard's condition.
    /// An element or a field of the binding counts as the binding. Nothing else copies it.
    #[test]
    fn test_an_argument_is_copied_only_where_a_later_operand_may_change_its_binding() {
        let src = "module m
pub struct P {
  xs: [2]u8
}
fn first(xs: [2]u8, k: u8) -> u8 = xs[0] + k
fn bump(mut xs: [2]u8) -> u8 {
  xs[0] = 7
  0
}
fn bump_rows(mut m: [2][2]u8) -> u8 {
  m[0][0] = 7
  0
}
fn bump_p(mut p: P) -> u8 {
  p.xs[0] = 7
  0
}
fn id(k: u8) -> u8 = k
pub fn read_later(c: bool) -> u8 {
  val xs: [2]u8 = [1, 2]
  first(xs, id(xs[0]))
}
pub fn other_binding(c: bool) -> u8 {
  val xs: [2]u8 = [1, 2]
  var ys: [2]u8 = [3, 4]
  first(xs, bump(mut ys))
}
pub fn in_an_operator(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, 0 + bump(mut xs))
}
pub fn in_an_index(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  val ys: [2]u8 = [3, 4]
  first(xs, ys[bump(mut xs) % 2])
}
pub fn if_unchanged(c: bool) -> u8 {
  val xs: [2]u8 = [1, 2]
  first(xs, if c { 1 } else { 2 })
}
pub fn if_assigns(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, if c {
    xs[0] = 7
    0
  } else {
    0
  })
}
pub fn assigned_value(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  var ys: [2]u8 = [3, 4]
  first(xs, if c {
    ys[0] = bump(mut xs)
    ys[0]
  } else {
    0
  })
}
pub fn assigned_place(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  var ys: [2]u8 = [3, 4]
  first(xs, if c {
    ys[bump(mut xs) % 2] = 0
    ys[0]
  } else {
    0
  })
}
pub fn loop_condition(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, if c {
    while bump(mut xs) > 5 {
    }
    0
  } else {
    0
  })
}
pub fn loop_body(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, if c {
    var i: u8 = 0
    while i < 1 {
      xs[0] = 7
      i = 1
    }
    0
  } else {
    0
  })
}
pub fn guard_condition(c: bool) -> u8 {
  var xs: [2]u8 = [1, 2]
  first(xs, if c {
    guard bump(mut xs) == 0 else {
      return 1
    }
    0
  } else {
    0
  })
}
pub fn an_element(c: bool) -> u8 {
  var m: [2][2]u8 = [[1, 2], [3, 4]]
  first(m[0], bump_rows(mut m))
}
pub fn a_field(c: bool) -> u8 {
  var p = P(xs: [1, 2])
  first(p.xs, bump_p(mut p))
}
pub fn changed_by_itself(c: bool) -> u8 {
  var m: [2][2]u8 = [[1, 2], [3, 4]]
  first(m[bump_rows(mut m) % 2], 0)
}
pub fn an_element_kept(c: bool) -> u8 {
  val m: [2][2]u8 = [[1, 2], [3, 4]]
  first(m[0], id(0))
}
pub fn a_field_kept(c: bool) -> u8 {
  val p = P(xs: [1, 2])
  first(p.xs, id(0))
}
pub fn indexed_kept(c: bool) -> u8 {
  val m: [2][2]u8 = [[1, 2], [3, 4]]
  first(m[id(1) % 2], 0)
}
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let count = |name: &str, copy: bool| {
            let f = lowered
                .module
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap();
            f.ops
                .iter()
                .filter(|op| {
                    if copy {
                        matches!(op, Op::Copy { .. })
                    } else {
                        matches!(op, Op::Check { .. })
                    }
                })
                .count()
        };
        // Each binding's literal is copied into it; the argument is copied once more where it
        // may change.
        for (name, bindings, kept) in [
            ("read_later", 1, false),
            ("other_binding", 2, false),
            ("in_an_operator", 1, true),
            ("in_an_index", 2, true),
            ("if_unchanged", 1, false),
            ("if_assigns", 1, true),
            ("assigned_value", 2, true),
            ("assigned_place", 2, true),
            ("loop_condition", 1, true),
            ("loop_body", 1, true),
            ("guard_condition", 1, true),
        ] {
            assert_eq!(count(name, true), bindings + usize::from(kept), "{}", name);
        }
        // An element or a field read from the binding is copied as the binding is: one copy
        // more than the same call whose later operand changes nothing.
        assert_eq!(
            count("an_element", true),
            count("an_element_kept", true) + 1
        );
        assert_eq!(count("a_field", true), count("a_field_kept", true) + 1);
        // An index whose own operand changes the array copies the array before indexing it,
        // and the element it gives is not copied again: no later operand changes the copy.
        assert_eq!(
            count("changed_by_itself", true),
            count("indexed_kept", true) + 1
        );
    }

    /// A struct's fields are checked against their refinements where a value enters from
    /// outside, a function's parameters, and not where a `mut` argument comes back from a call:
    /// the callee's stores into its fields are checked where they are made.
    #[test]
    fn test_a_structs_fields_are_checked_where_it_enters_and_not_when_it_comes_back() {
        let src = "module m
pub struct R {
  a: u8[0..10]
}
fn set(mut r: R) -> u8 {
  r.a = 3
  0
}
pub fn passes(mut r: R) -> u8 = set(mut r)
";
        let lowered = lower(&mente_sema::check_program_without_proofs(src).unwrap()).unwrap();
        let checks = |name: &str| {
            let f = lowered
                .module
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap();
            f.ops
                .iter()
                .filter(|op| matches!(op, Op::Check { .. }))
                .count()
        };
        // The entry, which takes `r` from outside, checks `r.a`; the function itself, after
        // `set` gives `r` back, checks nothing.
        assert!(checks("enter passes") > 0);
        assert_eq!(checks("passes"), 0);
    }

    #[test]
    fn test_machine_types_are_the_narrowest_that_hold_a_range() {
        let n = Num::from_i128;
        assert_eq!(machine(n(-1), n(5)), Some(IntTy::new(64, true)));
        assert_eq!(
            machine(n(0), Num::new(false, u128::from(u64::MAX))),
            Some(IntTy::new(64, false))
        );
        assert_eq!(
            machine(n(-1), Num::new(false, u128::from(u64::MAX))),
            Some(IntTy::new(128, true))
        );
        assert_eq!(
            machine(n(0), Num::new(false, u128::MAX)),
            Some(IntTy::new(128, false))
        );
        assert_eq!(machine(n(-1), Num::new(false, u128::MAX)), None);
        assert_eq!(
            common(IntTy::new(8, false), IntTy::new(64, true)),
            Some(IntTy::new(64, true))
        );
        assert_eq!(
            common(IntTy::new(64, false), IntTy::new(8, true)),
            Some(IntTy::new(128, true))
        );
        assert_eq!(common(IntTy::new(128, false), IntTy::new(8, true)), None);
        assert_eq!(
            common(IntTy::new(16, true), IntTy::new(32, true)),
            Some(IntTy::new(32, true))
        );
    }

    /// The kinds of the checks in the lowered function `name`, sorted: each `Check`'s trap kind,
    /// and `checked` for an operation that traps by itself.
    /// A literal's negation is a constant in MIR, on a literal without a suffix and with one of
    /// every type but `u128`, whose negation is an instruction the operator checks.
    #[test]
    fn test_a_literals_negation_is_a_constant_but_on_u128() {
        let src = "module m
pub fn bare(x: u8) -> i16 = x + -5
pub fn on_u8(x: u8) -> i16 = x + -5_u8
pub fn on_u64(x: u64) -> i128 = x + -5_u64
pub fn on_i8(x: i8) -> i16 = x + -5_i8
pub fn on_i128(x: i128) -> i128 = x + -5_i128
pub fn on_u128() -> u128 = -0_u128
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let negations = |name: &str| {
            let f = lowered.module.functions.iter().find(|f| f.name == name);
            f.unwrap()
                .ops
                .iter()
                .filter(|op| matches!(op, Op::Neg { .. }))
                .count()
        };
        for (name, n) in [
            ("bare", 0),
            ("on_u8", 0),
            ("on_u64", 0),
            ("on_i8", 0),
            ("on_i128", 0),
            ("on_u128", 1),
        ] {
            assert_eq!(negations(name), n, "{}", name);
        }
    }

    fn checks(lowered: &Lowered, name: &str) -> Vec<String> {
        let f = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no function {name}"));
        let mut kinds: Vec<String> = f
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Check { trap } => Some(f.traps[trap.0 as usize].kind.clone()),
                Op::Arith { trap: Some(_), .. } | Op::Neg { trap: Some(_) } | Op::Shl { .. } => {
                    Some("checked".to_string())
                }
                Op::Param
                | Op::Int(_)
                | Op::Bool(_)
                | Op::Arith { trap: None, .. }
                | Op::Neg { trap: None }
                | Op::Bits(_)
                | Op::Not
                | Op::Shr
                | Op::Cmp(_)
                | Op::Cast
                | Op::Slot { .. }
                | Op::Elem { .. }
                | Op::Load
                | Op::Store
                | Op::Copy { .. }
                | Op::Call(_) => None,
            })
            .collect();
        kinds.sort();
        kinds
    }

    /// Where Tier 1 proves a narrowing, an index, a divisor, a shift amount or a 128-bit
    /// operation, its check is left out of MIR; where it does not, the check stays, and so does
    /// every check of an entry, whose arguments come from outside the program, already narrowed.
    #[test]
    fn test_tier1_proofs_leave_bounds_and_overflow_checks_out_of_mir() {
        let src = "module m
pub fn get(xs: [4]u8, i: usize[0..4]) -> u8 = xs[i]
pub fn any(xs: [4]u8, i: usize) -> u8 = xs[i]
pub fn half(x: u8) -> u8 = x / 2
pub fn div(x: u8, y: u8) -> u8 = x / y
pub fn sh(x: u8, k: u8[0..8]) -> u8 = x >> k
pub fn shk(x: u8, k: u8) -> u8 = x >> k
pub fn sum(a: u8[0..100], b: u8[0..100]) -> u8 = a + b
pub fn add(a: u8, b: u8) -> u8 = a + b
pub fn big(a: i128[0..10], b: i128[0..10]) -> i128 = a * b
pub fn huge(a: i128, b: i128) -> i128 = a * b
pub fn mix(xs: [4]u8, i: usize, k: usize[0..4]) -> u8 {
  val a: u8 = xs[k]
  val b: u8 = xs[i]
  a & b
}
pub fn mask(a: u8[0..100], b: u8) -> u8 = (a + 1) & b
pub fn mask2(a: u8, b: u8) -> u8 = (a + 1) & b
pub fn arr(x: u8[0..10]) -> [2]u8 = [x + 1, 2]
pub fn rep(x: u8[0..10]) -> [3]u8 = [x + 1; 3]
fn take(v: u8) -> u8 = v
pub fn pass(x: u8[0..10]) -> u8 = take(x + 1)
pub fn pass2(x: u8) -> u8 = take(x + 1)
";
        let proven = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let unproven = lower(&mente_sema::check_program_without_proofs(src).unwrap()).unwrap();
        let both = |name: &str| (checks(&proven, name), checks(&unproven, name));
        let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let index = "index-out-of-bounds";
        for (name, kept, all) in [
            ("get", &[][..], &[index][..]),
            ("any", &[index], &[index]),
            ("half", &[], &["division-by-zero", "narrowing"]),
            (
                "div",
                &["division-by-zero", "narrowing"],
                &["division-by-zero", "narrowing"],
            ),
            ("sh", &[], &["shift-amount"]),
            ("shk", &["shift-amount"], &["shift-amount"]),
            ("sum", &[], &["narrowing"]),
            ("add", &["narrowing"], &["narrowing"]),
            ("big", &[], &["checked"]),
            ("huge", &["checked"], &["checked"]),
            ("mix", &[index], &[index, index]),
            ("mask", &[], &["narrowing"]),
            ("mask2", &["narrowing"], &["narrowing"]),
            ("arr", &[], &["narrowing", "narrowing"]),
            ("rep", &[], &["narrowing"]),
            ("pass", &[], &["narrowing"]),
            ("pass2", &["narrowing"], &["narrowing"]),
        ] {
            assert_eq!(both(name), (kinds(kept), kinds(all)), "{name}");
        }
        // An entry takes its arguments already narrowed into their types, by whoever calls it.
        assert_eq!(checks(&proven, "enter half"), kinds(&[]));
    }

    /// Where Tier 1 proves a refinement, a `requires` or `ensures` clause, a loop invariant or
    /// a `decreases` measure, its check is left out of MIR, clause by clause: a clause outside
    /// the Tier-1 fragment, which it does not prove, keeps its check, and so does an entry.
    #[test]
    fn test_tier1_proofs_leave_contract_checks_out_of_mir() {
        let src = "module m
fn need(x: u8) -> u8
  requires x > 2
= x - 1
pub fn use_need(x: u8[5..10]) -> u8 = need(x)
fn small(k: u8[0..10]) -> u8 = k
pub fn give(x: u8[0..5]) -> u8 = small(x)
pub fn land(x: u8[0..5]) -> u8[0..10] = x
fn bump(mut k: u8[0..10]) {
  guard k < 9 else {
    return
  }
  k += 1
}
pub fn twice(x: u8[0..5]) -> u8 {
  var k: u8[0..10] = x
  bump(mut k)
  k
}
pub fn count(n: u8) -> u8 {
  var i: u8 = 0
  while i < n invariant i <= n and n >= 0 decreases n - i {
    i += 1
  }
  i
}
fn down(n: u8) -> u8
  decreases n
= if n == 0 { 0 } else { down(n - 1) }
pub fn start(n: u8) -> u8 = down(n)
pub fn pos(x: u8) -> u8
  ensures result >= x
= x
@checked
fn both(x: u8, y: u8) -> u8
  requires x * y < 100
  requires x < 50
= x
pub fn call_both(x: u8[0..5], y: u8[0..5]) -> u8 = both(x, y)
";
        let proven = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let unproven = lower(&mente_sema::check_program_without_proofs(src).unwrap()).unwrap();
        let both = |name: &str| (checks(&proven, name), checks(&unproven, name));
        let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (refinement, decreases) = ("refinement", "decreases");
        for (name, kept, all) in [
            ("use_need", &[][..], &["requires"][..]),
            ("give", &[], &[refinement]),
            ("land", &[], &[refinement]),
            ("twice", &[], &[refinement, refinement, refinement]),
            (
                "count",
                &[],
                &[decreases, decreases, "invariant", "narrowing", "narrowing"],
            ),
            (
                "down",
                &[],
                &[decreases, decreases, "narrowing", "narrowing"],
            ),
            ("start", &[], &[decreases]),
            ("pos", &[], &["ensures"]),
            ("call_both", &["requires"], &["requires", "requires"]),
        ] {
            assert_eq!(both(name), (kinds(kept), kinds(all)), "{name}");
        }
        // An entry checks its arguments' refinements whatever Tier 1 proves; their narrowing
        // into their types is its caller's.
        assert_eq!(checks(&proven, "enter give"), kinds(&[refinement]));
    }
}

//! The encoding of a function: its paths followed from its entry, each value a term over its
//! inputs, in the order the reference interpreter evaluates (`spec/core-calculus.md`, items 4,
//! 6, 7 and 10).
//!
//! A path is a condition on the inputs, `active`: it holds of the inputs that run the program to
//! this point without a trap. A check of the function becomes a [`Site`] whose condition is the
//! path and the check's failure, and then a conjunct of the path, since a failed check ends its
//! run. Where control splits, at an `if`, a `match`, `and` and `or`, each branch runs on its own
//! path and the bindings join after it as `ite` terms of the condition that chose. A `return`, a
//! `break` and a `continue` keep the path and the bindings they leave with, which join where
//! control resumes. A call runs the callee's body in a frame of its own; its checks end its
//! caller's paths too, but only the encoded function's own are sites. Each call is counted
//! against the stack budget before its arguments are narrowed (item 25): where the frames of the
//! calls begun and not returned and the callee's together exceed it, every run that reaches the
//! call traps there, and its path ends, neither a site nor a cut.

use std::collections::BTreeMap;

use mente_syntax::tree::{Arg, Bound};
use mente_syntax::tree::{
    Arm, ArmBody, AssignOp, BinaryOp, BlockId, Body, ContractKind, Else, ExprId, ExprKind,
    PatternKind, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

use crate::num::Big;
use crate::term::{self, Bits, Sort, Terms, T};
use crate::{
    Encoding, Function, IntType, Kind, Module, Param, Shape, Site, Tag, Trap, I64, ROUNDS,
};

/// The most terms one function's encoding makes before it is cut off as too large.
pub const TERMS: usize = 200_000;

/// A value: an integer, a boolean or an array as a term, or no value.
#[derive(Clone, Debug)]
enum V {
    Int(T),
    Bool(T),
    /// An array, with its shape, a `Shape::Array`.
    Arr(T, Shape),
    Unit,
}

type Vars = BTreeMap<String, V>;

/// One call of a function: its bindings and what leaves it.
#[derive(Clone, Debug)]
struct Frame<'m> {
    func: usize,
    f: &'m Function<'m>,
    vars: Vars,
    /// Each binding's declared type in the function's tree, where it states one.
    declared: BTreeMap<String, Option<TypeId>>,
    /// The `decreases` measure on entry, for the call of itself to compare with.
    measure: Option<Vec<T>>,
    /// Each way out of the call: its path, its value and its bindings.
    returns: Vec<(T, V, Vars)>,
    /// The ways out of each loop the call is in, the innermost last.
    loops: Vec<Exits>,
}

#[derive(Clone, Debug, Default)]
struct Exits {
    breaks: Vec<(T, Vars)>,
    continues: Vec<(T, Vars)>,
}

/// Where a check stands: the function whose obligation it is, the place Tier 1 records it at,
/// and the place the interpreter traps at.
#[derive(Clone, Copy, Debug)]
struct At {
    owner: usize,
    key: Span,
    trap: Span,
}

/// An index of a place being assigned: its value, where Tier 1 and the interpreter report it,
/// and the length of the array it indexes.
#[derive(Clone, Copy, Debug)]
struct Step {
    i: T,
    key: Span,
    trap: Span,
    len: u128,
}

/// The places of an operator's check: the whole operation, and its left and right operands;
/// and whether the shifted operand of a shift has one type, which Tier 1 then names in the tag of
/// the shift's narrowing.
#[derive(Clone, Copy, Debug)]
struct Places {
    whole: Span,
    left: Span,
    right: Span,
    one: bool,
}

struct Exec<'m> {
    module: &'m Module<'m>,
    entry: usize,
    t: Terms,
    active: T,
    frame: Frame<'m>,
    callers: Vec<Frame<'m>>,
    sites: Vec<Site>,
    unencoded: Vec<(Span, &'static str)>,
    /// Whether checks are sites: not on the entry, whose checks the inputs are taken to pass.
    recording: bool,
    /// The paths cut off past the bounds.
    cut: Vec<T>,
    /// The paths cut off at a construct the encoding lacks.
    lacked: Vec<T>,
    exhausted: bool,
    /// The most terms the encoding makes.
    terms: usize,
}

/// The shape of a type expression of `tree`.
fn shape_of(tree: &Tree, ty: TypeId) -> Shape {
    match &tree.ty(ty).kind {
        TypeKind::Int { name, .. } => IntType::named(name).map_or(Shape::Other, Shape::Int),
        TypeKind::Bool => Shape::Bool,
        TypeKind::Array { len, elem } => Shape::Array(*len, Box::new(shape_of(tree, *elem))),
        TypeKind::Named(_) | TypeKind::Enum(_) | TypeKind::Error => Shape::Other,
    }
}

/// Whether a shape holds integers, booleans and arrays of them alone.
fn plain(s: &Shape) -> bool {
    match s {
        Shape::Int(_) | Shape::Bool => true,
        Shape::Array(_, e) => plain(e),
        Shape::Untyped | Shape::Other => false,
    }
}

/// Whether a type expression states a refinement anywhere within it.
fn refined_type(tree: &Tree, ty: TypeId) -> bool {
    match &tree.ty(ty).kind {
        TypeKind::Int { refinement, .. } => refinement.is_some(),
        TypeKind::Array { elem, .. } => refined_type(tree, *elem),
        TypeKind::Bool | TypeKind::Named(_) | TypeKind::Enum(_) | TypeKind::Error => false,
    }
}

/// Where Tier 1 records the checks of a narrowing: at an expression, and, where it reads an
/// array literal element by element as it lands in a binding, a place or a result, at each of its
/// elements; an argument is narrowed as one value, at its own place.
#[derive(Clone, Copy, Debug)]
struct Src<'m> {
    at: Option<(&'m Tree, ExprId)>,
    items: bool,
}

impl<'m> Src<'m> {
    fn none() -> Src<'m> {
        Src {
            at: None,
            items: false,
        }
    }

    /// The narrowing of `e`, read element by element where `items`.
    fn of(tree: &'m Tree, e: ExprId, items: bool) -> Src<'m> {
        Src {
            at: Some((tree, e)),
            items,
        }
    }

    /// The place of the checks, or `trap` where there is no expression.
    fn key(self, trap: Span) -> Span {
        self.at.map_or(trap, |(tree, e)| tree.expr(e).span)
    }

    /// Where element `k` is recorded: an array literal's element, a repeat's value, where
    /// Tier 1 reads elements one by one; the same place otherwise.
    fn element(self, k: u128) -> Src<'m> {
        let (Some((tree, e)), true) = (self.at, self.items) else {
            return self;
        };
        match &tree.expr(e).kind {
            ExprKind::Array(items) => usize::try_from(k)
                .ok()
                .and_then(|k| items.get(k))
                .map_or(self, |&i| Src::of(tree, i, true)),
            ExprKind::Repeat { value, .. } => Src::of(tree, *value, true),
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
            | ExprKind::Error => self,
        }
    }

    /// The distinct places of the elements: each element of an array literal, a repeat's value,
    /// or the place itself.
    fn places(self) -> Vec<Src<'m>> {
        let (Some((tree, e)), true) = (self.at, self.items) else {
            return vec![self];
        };
        match &tree.expr(e).kind {
            ExprKind::Array(items) => items.iter().map(|&i| Src::of(tree, i, true)).collect(),
            ExprKind::Repeat { .. } => vec![self.element(0)],
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
            | ExprKind::Error => vec![self],
        }
    }
}

/// The conjuncts of `e`, through every `and` at its top.
fn conjuncts(tree: &Tree, e: ExprId, out: &mut Vec<ExprId>) {
    match &tree.expr(e).kind {
        ExprKind::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
        } => {
            conjuncts(tree, *lhs, out);
            conjuncts(tree, *rhs, out);
        }
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
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => out.push(e),
    }
}

/// The expression whose value block `b` gives, its last statement's, if it is one.
fn tail(tree: &Tree, b: BlockId) -> Option<ExprId> {
    let s = *tree.block(b).stmts.last()?;
    match &tree.stmt(s).kind {
        StmtKind::Expr(e) => Some(*e),
        StmtKind::Binding { .. }
        | StmtKind::Assign { .. }
        | StmtKind::While { .. }
        | StmtKind::Guard { .. }
        | StmtKind::Return(_)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Discard(_) => None,
    }
}

/// Where a block body ends, as the interpreter reports it: its last statement, or the block.
fn end_of(tree: &Tree, b: BlockId) -> Span {
    tree.block(b)
        .stmts
        .last()
        .map_or(tree.block(b).span, |&s| tree.stmt(s).span)
}

/// Encodes function `entry` of `module`, which has a body. A function of none, or with a
/// parameter the encoding lacks, has no sites, and lists why.
pub fn encode<'m>(module: &'m Module<'m>, entry: usize) -> Encoding {
    encode_within(module, entry, TERMS)
}

/// [`encode`], cut off as too large once it has made `terms` terms.
pub fn encode_within<'m>(module: &'m Module<'m>, entry: usize, terms: usize) -> Encoding {
    let mut t = Terms::default();
    let tru = t.bool(true);
    let fals = t.bool(false);
    let Some(f) = module.functions.get(entry).and_then(Option::as_ref) else {
        return Encoding {
            function: String::new(),
            terms: t,
            params: Vec::new(),
            sites: Vec::new(),
            unencoded: Vec::new(),
            admitted: fals,
            cut: tru,
            cut_past_bound: false,
            cut_at_lacked: true,
            returns: fals,
            result: None,
            finals: Vec::new(),
        };
    };
    let mut x = Exec {
        module,
        entry,
        t,
        active: tru,
        frame: Frame {
            func: entry,
            f,
            vars: Vars::new(),
            declared: BTreeMap::new(),
            measure: None,
            returns: Vec::new(),
            loops: Vec::new(),
        },
        callers: Vec::new(),
        sites: Vec::new(),
        unencoded: Vec::new(),
        recording: false,
        cut: Vec::new(),
        lacked: Vec::new(),
        exhausted: false,
        terms,
    };
    let tree = &f.syntax.tree;
    let mut params = Vec::new();
    let mut values = Vec::new();
    for p in &f.syntax.params {
        let shape = shape_of(tree, p.ty);
        if !plain(&shape) {
            x.unencoded
                .push((p.name_span, "a parameter of a struct or an enum"));
        } else if shape.elements().is_none() {
            x.unencoded.push((
                p.name_span,
                "an array of more elements than an encoding holds",
            ));
        }
        if !x.unencoded.is_empty() {
            continue;
        }
        let mut inputs = Vec::new();
        let v = x.input(&format!("p.{}", p.name), &shape, &mut inputs);
        params.push(Param {
            name: p.name.clone(),
            shape,
            inputs,
        });
        values.push(v);
    }
    if !x.unencoded.is_empty() {
        let fals = x.t.bool(false);
        return Encoding {
            function: f.syntax.name.clone(),
            terms: x.t,
            params,
            sites: Vec::new(),
            unencoded: x.unencoded,
            admitted: fals,
            cut: tru,
            cut_past_bound: false,
            cut_at_lacked: true,
            returns: fals,
            result: None,
            finals: Vec::new(),
        };
    }
    // The entry: the inputs lie in their types and refinements, and the `requires` and the
    // measure hold, as Tier 1 takes them.
    x.recording = true;
    x.enter(values, None, entry);
    let admitted = x.active;
    // A call from outside the program counts the function's own frame against an empty stack:
    // a frame past the budget traps at the function's name, whatever the arguments (item 25).
    x.overflows(entry, false);
    x.run_body();
    let mut cuts = x.cut.clone();
    cuts.extend(x.lacked.iter().copied());
    let cut = x.t.or(&cuts);
    let (cut_past_bound, cut_at_lacked) = (live(&x.t, &x.cut), live(&x.t, &x.lacked));
    // What the run returns: its value and each parameter's final value, joined over the ways
    // out of the call, and whether one of them holds.
    let ways: Vec<(T, (V, Vars))> = x
        .frame
        .returns
        .iter()
        .map(|(g, v, vars)| (*g, (v.clone(), vars.clone())))
        .collect();
    let guards: Vec<T> = ways.iter().map(|w| w.0).collect();
    let returns = x.t.or(&guards);
    let result = term(&x.select(&ways, |(v, _)| v.clone()));
    let finals = f
        .syntax
        .params
        .iter()
        .map(|p| {
            let name = p.name.clone();
            let back = x.select(&ways, |(_, vars)| {
                vars.get(&name).cloned().unwrap_or(V::Unit)
            });
            term(&back)
        })
        .collect();
    Encoding {
        function: f.syntax.name.clone(),
        terms: x.t,
        params,
        sites: x.sites,
        unencoded: x.unencoded,
        admitted,
        cut,
        cut_past_bound,
        cut_at_lacked,
        returns,
        result,
        finals,
    }
}

impl<'m> Exec<'m> {
    fn tree(&self) -> &'m Tree {
        let f: &'m Function<'m> = self.frame.f;
        &f.syntax.tree
    }

    fn shape(&self, e: ExprId) -> Shape {
        let f: &'m Function<'m> = self.frame.f;
        f.facts
            .exprs
            .get(e.0 as usize)
            .cloned()
            .unwrap_or(Shape::Other)
    }

    /// Whether the integers of `e` have different types, which one standard type holds.
    fn mixed(&self, e: ExprId) -> bool {
        let f: &'m Function<'m> = self.frame.f;
        f.facts.mixed.get(e.0 as usize).copied().unwrap_or(false)
    }

    /// The type the literals of `e` take, where it is literals alone.
    fn literal(&self, e: ExprId) -> Option<IntType> {
        let f: &'m Function<'m> = self.frame.f;
        f.facts.literals.get(e.0 as usize).copied().flatten()
    }

    fn span(&self, e: ExprId) -> Span {
        self.tree().expr(e).span
    }

    fn here(&self, key: Span, trap: Span) -> At {
        At {
            owner: self.frame.func,
            key,
            trap,
        }
    }

    /// The inputs of a parameter of shape `shape`, named from `name`: one, or one for each
    /// element of an array, named by its indexes.
    fn input(&mut self, name: &str, shape: &Shape, names: &mut Vec<String>) -> V {
        match shape {
            Shape::Int(_) | Shape::Untyped => {
                names.push(name.to_string());
                V::Int(self.t.input(name.to_string(), Sort::Int))
            }
            Shape::Bool => {
                names.push(name.to_string());
                V::Bool(self.t.input(name.to_string(), Sort::Bool))
            }
            Shape::Array(len, elem) => {
                let V::Arr(mut a, _) = self.default(shape) else {
                    return V::Unit;
                };
                for k in 0..*len {
                    let v = self.input(&format!("{}.{}", name, k), elem, names);
                    if let Some(x) = term(&v) {
                        let i = self.t.num(Big::from_u128(k));
                        a = self.t.store(a, i, x);
                    }
                }
                V::Arr(a, shape.clone())
            }
            Shape::Other => V::Unit,
        }
    }

    /// The value no input gives, of a shape: what a path that leaves or is cut off holds.
    fn default(&mut self, s: &Shape) -> V {
        match s {
            Shape::Int(_) | Shape::Untyped => V::Int(self.t.int(0)),
            Shape::Bool => V::Bool(self.t.bool(false)),
            Shape::Array(_, elem) => match self.default(elem) {
                V::Int(x) | V::Bool(x) | V::Arr(x, _) => V::Arr(self.t.konst(x), s.clone()),
                V::Unit => V::Unit,
            },
            Shape::Other => V::Unit,
        }
    }

    fn int(&mut self, v: &V) -> T {
        match v {
            V::Int(x) => *x,
            V::Bool(_) | V::Arr(..) | V::Unit => self.t.int(0),
        }
    }

    fn boolean(&mut self, v: &V) -> T {
        match v {
            V::Bool(x) => *x,
            V::Int(_) | V::Arr(..) | V::Unit => self.t.bool(false),
        }
    }

    /// A term of the shape `s` as a value.
    fn wrap(&self, x: T, s: &Shape) -> V {
        match s {
            Shape::Int(_) | Shape::Untyped => V::Int(x),
            Shape::Bool => V::Bool(x),
            Shape::Array(..) => V::Arr(x, s.clone()),
            Shape::Other => V::Unit,
        }
    }

    fn in_range(&mut self, x: T, t: IntType) -> T {
        let lo = self.t.num(t.min());
        let hi = self.t.num(t.max());
        let a = self.t.le(lo, x);
        let b = self.t.le(x, hi);
        self.t.and2(a, b)
    }

    /// A check that `ok` holds here: a site where it is the encoded function's, and then a
    /// conjunct of the path.
    fn check(&mut self, ok: T, kind: Kind, tag: Tag, at: At, trap: &'static str) {
        if self.recording && at.owner == self.entry {
            let fails = self.t.not(ok);
            let fails = self.t.and2(self.active, fails);
            let (beyond_bound, beyond_lacked) =
                (live(&self.t, &self.cut), live(&self.t, &self.lacked));
            self.sites.push(Site {
                kind,
                span: at.key,
                tag,
                trap: Trap {
                    kind: trap,
                    span: at.trap,
                },
                fails,
                beyond_bound,
                beyond_lacked,
            });
        }
        self.active = self.t.and2(self.active, ok);
    }

    /// Cuts the path off at a construct the encoding lacks.
    fn lacks(&mut self, span: Span, what: &'static str) {
        if !self.unencoded.contains(&(span, what)) {
            self.unencoded.push((span, what));
        }
        self.lacked.push(self.active);
        self.active = self.t.bool(false);
    }

    /// Whether a call of `callee` runs out of stack (`spec/core-calculus.md`, item 25): the
    /// frames of the calls begun and not returned, if `nested`, and the callee's together exceed
    /// the budget. The frames are the checker's and a path's calls are known, so where they do,
    /// every run that reaches the call traps there: the path ends, neither a site nor a cut.
    fn overflows(&mut self, callee: usize, nested: bool) -> bool {
        let frame = |i: usize| {
            self.module
                .functions
                .get(i)
                .and_then(Option::as_ref)
                .map_or(0, |f| f.frame)
        };
        let in_use: u128 = if nested {
            self.callers
                .iter()
                .chain(std::iter::once(&self.frame))
                .map(|fr| frame(fr.func))
                .sum()
        } else {
            0
        };
        if in_use.saturating_add(frame(callee)) > self.module.budget {
            self.active = self.t.bool(false);
            return true;
        }
        false
    }

    /// Cuts the path off past the bounds.
    fn bound(&mut self) {
        self.cut.push(self.active);
        self.active = self.t.bool(false);
    }

    /// `c ? a : b`, of two values of one kind; where one is no value, the other.
    fn join(&mut self, c: T, a: V, b: V) -> V {
        match (a, b) {
            (V::Int(x), V::Int(y)) => V::Int(self.t.ite(c, x, y)),
            (V::Bool(x), V::Bool(y)) => V::Bool(self.t.ite(c, x, y)),
            (V::Arr(x, s), V::Arr(y, _)) => {
                if self.t.sort(x) == self.t.sort(y) {
                    V::Arr(self.t.ite(c, x, y), s)
                } else {
                    V::Arr(x, s)
                }
            }
            (V::Unit, v) | (v, V::Unit) => v,
            (v, V::Int(_) | V::Bool(_) | V::Arr(..)) => v,
        }
    }

    /// The bindings of two branches, joined by the condition that chose the first.
    fn join_vars(&mut self, c: T, a: Vars, mut b: Vars) -> Vars {
        let mut out = Vars::new();
        for (name, va) in a {
            let v = match b.remove(&name) {
                Some(vb) => self.join(c, va, vb),
                None => va,
            };
            out.insert(name, v);
        }
        out.extend(b);
        out
    }

    /// The values of exclusive paths, joined: each where its path holds.
    fn select<X: Clone>(&mut self, items: &[(T, X)], value: impl Fn(&X) -> V) -> V {
        let mut it = items.iter().rev();
        let Some((_, last)) = it.next() else {
            return V::Unit;
        };
        let mut acc = value(last);
        for (g, x) in it {
            acc = self.join(*g, value(x), acc);
        }
        acc
    }

    /// The bindings of exclusive paths, joined.
    fn select_vars(&mut self, items: &[(T, Vars)]) -> Vars {
        let mut it = items.iter().rev();
        let Some((_, last)) = it.next() else {
            return Vars::new();
        };
        let mut acc = last.clone();
        for (g, vars) in it {
            acc = self.join_vars(*g, vars.clone(), acc);
        }
        acc
    }

    /// The path of any of `items`.
    fn any(&mut self, items: &[T]) -> T {
        self.t.or(items)
    }

    // -- narrowing ---------------------------------------------------------------------------

    /// `v` narrowed into `plain`, then into the refinements `declared` states, as Tier 1
    /// records it at `src` and the interpreter traps at `trap`.
    #[allow(clippy::too_many_arguments)]
    fn narrow(
        &mut self,
        v: V,
        plain: &Shape,
        declared: Option<(&'m Tree, TypeId)>,
        src: Src<'m>,
        owner: usize,
        trap: Span,
        tag: Tag,
    ) -> V {
        self.narrow_plain(&v, plain, src, owner, trap);
        if let Some((tree, ty)) = declared {
            self.refined(&v, tree, ty, src, owner, trap, tag);
        }
        v
    }

    fn narrow_plain(&mut self, v: &V, plain: &Shape, src: Src<'m>, owner: usize, trap: Span) {
        let key = src.key(trap);
        match (plain, v) {
            (Shape::Int(t), V::Int(x)) => {
                let ok = self.in_range(*x, *t);
                let at = At { owner, key, trap };
                self.check(ok, Kind::Range, Tag::Into(t.name), at, "narrowing");
            }
            // An array's elements are narrowed one by one; an element that narrows nothing, a
            // `bool` or a struct, checks nothing, as a scalar of its shape does.
            (Shape::Array(len, elem), V::Arr(a, s)) => {
                let held = match s {
                    Shape::Array(_, e) => (**e).clone(),
                    Shape::Int(_) | Shape::Untyped | Shape::Bool | Shape::Other => (**elem).clone(),
                };
                // An array of no element holds nothing to check: the check holds, and stands
                // as the site of the obligation Tier 1 records.
                if *len == 0 {
                    self.held(plain, src, owner, trap);
                    return;
                }
                if plain.elements().is_none() {
                    // Every element of a repeat is its one value, narrowed once.
                    if let term::Node::Const(_, x) = self.t.node(*a).clone() {
                        let item = self.wrap(x, &held);
                        self.narrow_plain(&item, elem, src.element(0), owner, trap);
                        return;
                    }
                    // Every element of a value of the type already lies in it: the check
                    // holds, and stands as a site for the obligation Tier 1 records.
                    if s == plain {
                        self.held(plain, src, owner, trap);
                        return;
                    }
                    self.lacks(trap, "an array of more elements than an encoding holds");
                    return;
                }
                for k in 0..*len {
                    let i = self.t.num(Big::from_u128(k));
                    let x = self.t.select(*a, i);
                    let item = self.wrap(x, &held);
                    self.narrow_plain(&item, elem, src.element(k), owner, trap);
                }
            }
            // No value comes, as from a `todo` or an `if` whose every branch leaves: no path
            // reaches the narrowing, which stands as a site for the obligation Tier 1 records.
            (Shape::Int(_) | Shape::Array(..), V::Bool(_) | V::Arr(..) | V::Int(_) | V::Unit) => {
                self.held(plain, src, owner, trap);
            }
            (Shape::Untyped | Shape::Bool | Shape::Other, _) => {}
        }
    }

    /// The narrowing into `plain` as checks that hold, one at each place Tier 1 records it.
    fn held(&mut self, plain: &Shape, src: Src<'m>, owner: usize, trap: Span) {
        match plain {
            Shape::Int(t) => {
                let ok = self.t.bool(true);
                let at = At {
                    owner,
                    key: src.key(trap),
                    trap,
                };
                self.check(ok, Kind::Range, Tag::Into(t.name), at, "narrowing");
            }
            Shape::Array(_, elem) => {
                for each in src.places() {
                    self.held(elem, each, owner, trap);
                }
            }
            Shape::Untyped | Shape::Bool | Shape::Other => {}
        }
    }

    /// The refinements of type `ty` of `tree`, checked of `v`, their bounds read in this frame.
    #[allow(clippy::too_many_arguments)]
    fn refined(
        &mut self,
        v: &V,
        tree: &'m Tree,
        ty: TypeId,
        src: Src<'m>,
        owner: usize,
        trap: Span,
        tag: Tag,
    ) {
        let key = src.key(trap);
        let at = At { owner, key, trap };
        match (&tree.ty(ty).kind, v) {
            (
                TypeKind::Int {
                    refinement: Some(r),
                    ..
                },
                V::Int(x),
            ) => {
                let lo = self.bound_value(&r.lo);
                let hi = self.bound_value(&r.hi);
                let low = self.t.le(lo, *x);
                let high = if r.inclusive {
                    self.t.le(*x, hi)
                } else {
                    self.t.lt(*x, hi)
                };
                let ok = self.t.and2(low, high);
                self.check(ok, Kind::Refinement, tag, at, "refinement");
            }
            // No value comes: the refinement holds where no path reaches it.
            (
                TypeKind::Int {
                    refinement: Some(_),
                    ..
                },
                V::Bool(_) | V::Arr(..) | V::Unit,
            ) => {
                let ok = self.t.bool(true);
                self.check(ok, Kind::Refinement, tag, at, "refinement");
            }
            (TypeKind::Array { len, elem }, V::Arr(a, s)) => {
                if !refined_type(tree, *elem) {
                    return;
                }
                if *len == 0 {
                    for each in src.places() {
                        self.refined(&V::Unit, tree, *elem, each, owner, trap, tag);
                    }
                    return;
                }
                if shape_of(tree, ty).elements().is_none() {
                    self.lacks(trap, "an array of more elements than an encoding holds");
                    return;
                }
                let held = match s {
                    Shape::Array(_, e) => (**e).clone(),
                    Shape::Int(_) | Shape::Untyped | Shape::Bool | Shape::Other => Shape::Other,
                };
                for k in 0..*len {
                    let i = self.t.num(Big::from_u128(k));
                    let x = self.t.select(*a, i);
                    let item = self.wrap(x, &held);
                    self.refined(&item, tree, *elem, src.element(k), owner, trap, tag);
                }
            }
            (TypeKind::Array { elem, .. }, V::Int(_) | V::Bool(_) | V::Unit) => {
                if refined_type(tree, *elem) {
                    for each in src.places() {
                        self.refined(&V::Unit, tree, *elem, each, owner, trap, tag);
                    }
                }
            }
            (
                TypeKind::Int {
                    refinement: None, ..
                },
                _,
            )
            | (TypeKind::Bool | TypeKind::Named(_) | TypeKind::Enum(_) | TypeKind::Error, _) => {}
        }
    }

    /// A refinement's bound, in this frame.
    fn bound_value(&mut self, b: &Bound) -> T {
        match b {
            Bound::Int { negative, value } => {
                let v = Big::from_u128(*value);
                self.t.num(if *negative { v.neg() } else { v })
            }
            Bound::Name(n) => match self.frame.vars.get(n).cloned() {
                Some(V::Int(x)) => x,
                Some(V::Bool(_) | V::Arr(..) | V::Unit) | None => self.t.int(0),
            },
            Bound::Len(n) => match self.frame.vars.get(n) {
                Some(V::Arr(_, Shape::Array(len, _))) => {
                    let len = *len;
                    self.t.num(Big::from_u128(len))
                }
                Some(V::Arr(..) | V::Int(_) | V::Bool(_) | V::Unit) | None => self.t.int(0),
            },
        }
    }

    // -- calls -------------------------------------------------------------------------------

    /// Enters function `func` with its arguments' values, from the call at `call` of the
    /// function `caller`, or as the encoded function where `call` is `None`: each argument
    /// narrowed into its parameter's type, then their refinements, the `requires` and the
    /// measure, as the interpreter enters a call.
    fn enter(&mut self, values: Vec<V>, call: Option<(ExprId, &'m [Arg])>, func: usize) {
        let Some(f) = self.module.functions.get(func).and_then(Option::as_ref) else {
            return;
        };
        // On the encoded function's entry its inputs are taken to pass these checks, which are
        // its callers' obligations, not its own: they belong to no function. The checks within
        // its clauses, a divisor in a `requires` say, are its own.
        let caller = match call {
            Some(_) => self.frame.func,
            None => usize::MAX,
        };
        let caller_tree = self.tree();
        let tree = &f.syntax.tree;
        let call_span = call.map(|(e, _)| caller_tree.expr(e).span);
        let arg_src = |k: usize| -> Src<'m> {
            match call.and_then(|(_, args)| args.get(k)) {
                Some(a) => Src::of(caller_tree, a.value, false),
                None => Src::none(),
            }
        };
        let arg_span = |k: usize| arg_src(k).at.map(|(t, e)| t.expr(e).span);
        let mut bound = Vec::new();
        for (k, (p, v)) in f.syntax.params.iter().zip(values).enumerate() {
            let plain = shape_of(tree, p.ty);
            let trap = arg_span(k).unwrap_or(p.name_span);
            self.narrow_plain(&v, &plain, arg_src(k), caller, trap);
            bound.push(v);
        }
        let frame = Frame {
            func,
            f,
            vars: f
                .syntax
                .params
                .iter()
                .zip(&bound)
                .map(|(p, v)| (p.name.clone(), v.clone()))
                .collect(),
            declared: f
                .syntax
                .params
                .iter()
                .map(|p| (p.name.clone(), Some(p.ty)))
                .collect(),
            measure: None,
            returns: Vec::new(),
            loops: Vec::new(),
        };
        let caller_measure = self.frame.measure.clone();
        let before = std::mem::replace(&mut self.frame, frame);
        if call.is_some() {
            self.callers.push(before);
        }
        // A refinement may name another parameter, so each is checked once all are bound.
        for (k, (p, v)) in f.syntax.params.iter().zip(&bound).enumerate() {
            let trap = arg_span(k).unwrap_or(p.name_span);
            self.refined(v, tree, p.ty, arg_src(k), caller, trap, Tag::None);
        }
        let at_call = call_span.unwrap_or(f.syntax.name_span);
        for c in &f.syntax.contracts {
            if c.kind == ContractKind::Requires {
                let v = self.eval(c.expr);
                let ok = self.boolean(&v);
                let at = At {
                    owner: caller,
                    key: at_call,
                    trap: at_call,
                };
                let tag = Tag::Clause(tree.expr(c.expr).span);
                self.check(ok, Kind::Requires, tag, at, "requires");
            }
        }
        let measures: Vec<ExprId> = f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == ContractKind::Decreases)
            .map(|c| c.expr)
            .collect();
        if measures.is_empty() {
            return;
        }
        let mut values = Vec::new();
        for &m in &measures {
            let v = self.eval(m);
            let x = self.int(&v);
            let zero = self.t.int(0);
            let ok = self.t.le(zero, x);
            let at = At {
                owner: caller,
                key: at_call,
                trap: tree.expr(m).span,
            };
            let tag = Tag::Clause(tree.expr(m).span);
            self.check(ok, Kind::Decreases, tag, at, "decreases");
            values.push(x);
        }
        if let (Some(_), true, Some(before)) = (call, caller == func, caller_measure) {
            let ok = self.lex_less(&values, &before);
            let at = At {
                owner: caller,
                key: at_call,
                trap: at_call,
            };
            self.check(ok, Kind::Decreases, Tag::Smaller, at, "decreases");
        }
        self.frame.measure = Some(values);
    }

    /// Whether `a` is smaller than `b` in lexicographic order.
    fn lex_less(&mut self, a: &[T], b: &[T]) -> T {
        match (a.split_first(), b.split_first()) {
            (Some((x, xs)), Some((y, ys))) => {
                let less = self.t.lt(*x, *y);
                let same = self.t.eq(*x, *y);
                let rest = self.lex_less(xs, ys);
                let tie = self.t.and2(same, rest);
                self.t.or2(less, tie)
            }
            _ => self.t.bool(false),
        }
    }

    /// The body of the function of the current frame, which ends as a `return` of its value.
    fn run_body(&mut self) {
        let f: &'m Function<'m> = self.frame.f;
        let tree = &f.syntax.tree;
        match f.syntax.body {
            Some(Body::Expr(e)) => {
                let v = self.eval(e);
                let span = tree.expr(e).span;
                self.leave(v, Some(e), span, span);
            }
            Some(Body::Block(b)) => {
                let v = self.block(b);
                let trap = end_of(tree, b);
                let last = tail(tree, b);
                let key = last.map_or(trap, |e| tree.expr(e).span);
                self.leave(v, last, key, trap);
            }
            None => {}
        }
    }

    /// Leaves the call with `v`, given by `src`: narrowed into the result's type, then the
    /// `ensures` checked; Tier 1 records both at `key`, the interpreter traps at `trap`.
    fn leave(&mut self, v: V, src: Option<ExprId>, key: Span, trap: Span) {
        let f: &'m Function<'m> = self.frame.f;
        let tree = &f.syntax.tree;
        let func = self.frame.func;
        let v = match f.syntax.result {
            Some(t) => {
                let plain = shape_of(tree, t);
                let src = src.map_or(Src::none(), |e| Src::of(tree, e, true));
                self.narrow_plain(&v, &plain, src, func, trap);
                self.refined(&v, tree, t, src, func, trap, Tag::None);
                v
            }
            None => v,
        };
        for c in &f.syntax.contracts {
            if c.kind != ContractKind::Ensures {
                continue;
            }
            self.frame.vars.insert("result".to_string(), v.clone());
            let ok = self.eval(c.expr);
            let ok = self.boolean(&ok);
            let at = At {
                owner: func,
                key,
                trap,
            };
            let tag = Tag::Clause(tree.expr(c.expr).span);
            self.check(ok, Kind::Ensures, tag, at, "ensures");
        }
        let vars = self.frame.vars.clone();
        self.frame.returns.push((self.active, v, vars));
        self.active = self.t.bool(false);
    }

    /// The call at `e` of `callee`.
    fn call(&mut self, e: ExprId, callee: &str, args: &'m [Arg]) -> V {
        let span = self.span(e);
        let shape = self.shape(e);
        let index = self.module.index(callee);
        let Some(func) = index.filter(|&i| {
            self.module
                .functions
                .get(i)
                .is_some_and(|f| f.as_ref().is_some_and(|f| f.syntax.body.is_some()))
        }) else {
            self.lacks(span, "a call of a function without a body");
            return self.default(&shape);
        };
        // The arguments, left to right; a `mut` one is read from its place.
        let mut values = Vec::new();
        let mut places = Vec::new();
        for a in args {
            if a.is_mut {
                let (root, steps) = self.place(a.value);
                let v = self.read(&root, &steps);
                values.push(v);
                places.push(Some((root, steps)));
            } else {
                values.push(self.eval(a.value));
                places.push(None);
            }
        }
        // The call is counted against the stack budget before its arguments are narrowed.
        if self.overflows(func, true) {
            return self.default(&shape);
        }
        let depth = self
            .callers
            .iter()
            .chain(std::iter::once(&self.frame))
            .filter(|fr| fr.func == func)
            .count();
        if depth >= ROUNDS {
            self.bound();
            return self.default(&shape);
        }
        let caller_tree = self.tree();
        self.enter(values, Some((e, args)), func);
        self.run_body();
        let next = match self.callers.pop() {
            Some(f) => f,
            None => empty_frame(self.frame.f),
        };
        let done = std::mem::replace(&mut self.frame, next);
        let paths: Vec<T> = done.returns.iter().map(|r| r.0).collect();
        self.active = self.any(&paths);
        let ways: Vec<(T, (V, Vars))> = done
            .returns
            .iter()
            .map(|(g, v, vars)| (*g, (v.clone(), vars.clone())))
            .collect();
        let result = self.select(&ways, |(v, _)| v.clone());
        // Each `mut` place takes back what the callee left in its parameter.
        let names: Vec<String> = done
            .f
            .syntax
            .params
            .iter()
            .map(|p| p.name.clone())
            .collect();
        for (k, place) in places.into_iter().enumerate() {
            let Some((root, steps)) = place else {
                continue;
            };
            let name = names.get(k).cloned().unwrap_or_default();
            let back = self.select(&ways, |(_, vars)| {
                vars.get(&name).cloned().unwrap_or(V::Unit)
            });
            if let Some((tree, ty)) = self.declared_of(&root, steps.len()) {
                let src = args
                    .get(k)
                    .map_or(Src::none(), |a| Src::of(caller_tree, a.value, false));
                let owner = self.frame.func;
                self.refined(&back, tree, ty, src, owner, span, Tag::Back);
            }
            self.write(&root, &steps, back);
        }
        result
    }

    // -- places ------------------------------------------------------------------------------

    /// The root binding of a place and its indexes, evaluated left to right.
    fn place(&mut self, e: ExprId) -> (String, Vec<Step>) {
        let tree = self.tree();
        match &tree.expr(e).kind {
            ExprKind::Name(n) => (n.clone(), Vec::new()),
            ExprKind::Index { base, index } => {
                let (root, mut steps) = self.place(*base);
                let v = self.eval(*index);
                let i = self.int(&v);
                let len = match self.shape(*base) {
                    Shape::Array(len, _) => len,
                    Shape::Int(_) | Shape::Untyped | Shape::Bool | Shape::Other => 0,
                };
                steps.push(Step {
                    i,
                    key: tree.expr(*index).span,
                    trap: tree.expr(e).span,
                    len,
                });
                (root, steps)
            }
            ExprKind::Field { .. } => {
                self.lacks(tree.expr(e).span, "a field of a struct");
                (String::new(), Vec::new())
            }
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
            | ExprKind::Error => (String::new(), Vec::new()),
        }
    }

    /// The check of an index of a place.
    fn check_step(&mut self, s: &Step) {
        let zero = self.t.int(0);
        let len = self.t.num(Big::from_u128(s.len));
        let low = self.t.le(zero, s.i);
        let high = self.t.lt(s.i, len);
        let ok = self.t.and2(low, high);
        let at = self.here(s.key, s.trap);
        self.check(ok, Kind::Index, Tag::None, at, "index-out-of-bounds");
    }

    /// The value a place holds, each index checked in turn.
    fn read(&mut self, root: &str, steps: &[Step]) -> V {
        let mut v = self.frame.vars.get(root).cloned().unwrap_or(V::Unit);
        for s in steps {
            self.check_step(s);
            v = match v {
                V::Arr(a, Shape::Array(_, elem)) => {
                    let x = self.t.select(a, s.i);
                    self.wrap(x, &elem)
                }
                V::Arr(..) | V::Int(_) | V::Bool(_) | V::Unit => V::Unit,
            };
        }
        v
    }

    /// `new` written into the place.
    fn write(&mut self, root: &str, steps: &[Step], new: V) {
        let Some(cur) = self.frame.vars.get(root).cloned() else {
            return;
        };
        let v = self.write_at(cur, steps, new);
        self.frame.vars.insert(root.to_string(), v);
    }

    fn write_at(&mut self, cur: V, steps: &[Step], new: V) -> V {
        let Some((s, rest)) = steps.split_first() else {
            return match (cur, new) {
                (V::Arr(_, shape), V::Arr(x, _)) => V::Arr(x, shape),
                (_, new) => new,
            };
        };
        match cur {
            V::Arr(a, Shape::Array(len, elem)) => {
                let x = self.t.select(a, s.i);
                let inner = self.wrap(x, &elem);
                let updated = self.write_at(inner, rest, new);
                match term(&updated) {
                    Some(u) => V::Arr(self.t.store(a, s.i, u), Shape::Array(len, elem)),
                    None => V::Arr(a, Shape::Array(len, elem)),
                }
            }
            other @ (V::Arr(..) | V::Int(_) | V::Bool(_) | V::Unit) => other,
        }
    }

    /// The declared type of the place of `root` down `depth` indexes, where it states one.
    fn declared_of(&self, root: &str, depth: usize) -> Option<(&'m Tree, TypeId)> {
        let tree = self.tree();
        let mut ty = (*self.frame.declared.get(root)?)?;
        for _ in 0..depth {
            ty = match &tree.ty(ty).kind {
                TypeKind::Array { elem, .. } => *elem,
                TypeKind::Int { .. }
                | TypeKind::Bool
                | TypeKind::Named(_)
                | TypeKind::Enum(_)
                | TypeKind::Error => return None,
            };
        }
        Some((tree, ty))
    }

    // -- statements --------------------------------------------------------------------------

    fn block(&mut self, b: BlockId) -> V {
        let tree = self.tree();
        let mut last = V::Unit;
        for &s in &tree.block(b).stmts {
            last = self.stmt(s);
        }
        last
    }

    fn stmt(&mut self, s: StmtId) -> V {
        let tree = self.tree();
        let span = tree.stmt(s).span;
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => self.eval(*e),
            StmtKind::Discard(e) => {
                self.eval(*e);
                V::Unit
            }
            StmtKind::Binding { name, ty, init, .. } => {
                let v = self.eval(*init);
                let f: &'m Function<'m> = self.frame.f;
                let plain = f
                    .facts
                    .bindings
                    .get(s.0 as usize)
                    .cloned()
                    .flatten()
                    .or_else(|| ty.map(|t| shape_of(tree, t)))
                    .unwrap_or_else(|| self.shape(*init));
                let declared = ty.map(|t| (tree, t));
                let func = self.frame.func;
                let trap = tree.expr(*init).span;
                let v = self.narrow(
                    v,
                    &plain,
                    declared,
                    Src::of(tree, *init, true),
                    func,
                    trap,
                    Tag::None,
                );
                let v = match v {
                    V::Arr(x, _) => V::Arr(x, plain),
                    other @ (V::Int(_) | V::Bool(_) | V::Unit) => other,
                };
                self.frame.vars.insert(name.clone(), v);
                self.frame.declared.insert(name.clone(), *ty);
                V::Unit
            }
            StmtKind::Assign { place, op, value } => {
                self.assign(*place, *op, *value);
                V::Unit
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                self.while_loop(*cond, invariants, decreases, *body);
                V::Unit
            }
            StmtKind::Guard { cond, otherwise } => {
                let v = self.eval(*cond);
                let c = self.boolean(&v);
                let before = self.active;
                let saved = self.frame.vars.clone();
                let nc = self.t.not(c);
                self.active = self.t.and2(before, nc);
                self.block(*otherwise);
                let otherwise_end = self.active;
                let vars = std::mem::take(&mut self.frame.vars);
                let held = self.t.and2(before, c);
                self.active = self.t.or2(held, otherwise_end);
                self.frame.vars = self.join_vars(c, saved, vars);
                V::Unit
            }
            StmtKind::Return(value) => {
                let (v, at) = match value {
                    Some(e) => (self.eval(*e), tree.expr(*e).span),
                    None => (V::Unit, span),
                };
                self.leave(v, *value, at, at);
                V::Unit
            }
            StmtKind::Break => {
                let exit = (self.active, self.frame.vars.clone());
                if let Some(l) = self.frame.loops.last_mut() {
                    l.breaks.push(exit);
                }
                self.active = self.t.bool(false);
                V::Unit
            }
            StmtKind::Continue => {
                let exit = (self.active, self.frame.vars.clone());
                if let Some(l) = self.frame.loops.last_mut() {
                    l.continues.push(exit);
                }
                self.active = self.t.bool(false);
                V::Unit
            }
        }
    }

    fn assign(&mut self, place: ExprId, op: AssignOp, value: ExprId) {
        let tree = self.tree();
        let (root, steps) = self.place(place);
        let plain = self.shape(place);
        let declared = self.declared_of(&root, steps.len());
        let func = self.frame.func;
        let at_value = tree.expr(value).span;
        match op {
            AssignOp::Set => {
                let v = self.eval(value);
                let v = self.narrow(
                    v,
                    &plain,
                    declared,
                    Src::of(tree, value, true),
                    func,
                    at_value,
                    Tag::None,
                );
                for s in &steps {
                    self.check_step(s);
                }
                self.write(&root, &steps, v);
            }
            AssignOp::Compound(bop) => {
                let current = self.read(&root, &steps);
                let current = self.int(&current);
                let r = self.eval(value);
                let r = self.int(&r);
                let lt = plain.int();
                let rt = self.shape(value).int();
                let work = match bop {
                    BinaryOp::Shl | BinaryOp::Shr => lt.unwrap_or(I64),
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
                    | BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => lt.or(rt).unwrap_or(I64),
                };
                let places = Places {
                    whole: at_value,
                    left: tree.expr(place).span,
                    right: at_value,
                    one: true,
                };
                let new = self.arith(bop, current, r, (lt, rt), work, places);
                let v = self.narrow(
                    V::Int(new),
                    &plain,
                    declared,
                    Src::of(tree, value, true),
                    func,
                    at_value,
                    Tag::None,
                );
                self.write(&root, &steps, v);
            }
        }
    }

    /// `while cond { body }`, its rounds unrolled: at each head the invariants are checked and
    /// the condition evaluated, and a round that runs checks the measure first.
    fn while_loop(
        &mut self,
        cond: ExprId,
        invariants: &[ExprId],
        decreases: &[ExprId],
        body: BlockId,
    ) {
        let tree = self.tree();
        let mut exits: Vec<(T, Vars)> = Vec::new();
        let mut before: Option<Vec<T>> = None;
        for round in 0..=ROUNDS {
            for &inv in invariants {
                let mut parts = Vec::new();
                conjuncts(tree, inv, &mut parts);
                let clause = tree.expr(inv).span;
                for part in parts {
                    let v = self.eval(part);
                    let ok = self.boolean(&v);
                    let at = self.here(tree.expr(part).span, clause);
                    self.check(ok, Kind::Invariant, Tag::Clause(clause), at, "invariant");
                }
            }
            let v = self.eval(cond);
            let c = self.boolean(&v);
            let nc = self.t.not(c);
            let out = self.t.and2(self.active, nc);
            exits.push((out, self.frame.vars.clone()));
            self.active = self.t.and2(self.active, c);
            if round == ROUNDS {
                self.bound();
                break;
            }
            if let Some(&first) = decreases.first() {
                let mut values = Vec::new();
                for &m in decreases {
                    let v = self.eval(m);
                    let x = self.int(&v);
                    let zero = self.t.int(0);
                    let ok = self.t.le(zero, x);
                    let span = tree.expr(m).span;
                    let at = self.here(span, span);
                    self.check(ok, Kind::Decreases, Tag::Clause(span), at, "decreases");
                    values.push(x);
                }
                // The first round has no round before to compare with: the check holds, and
                // stands as the site of the obligation however few rounds the bounds let run.
                let ok = match &before {
                    Some(b) => self.lex_less(&values, b),
                    None => self.t.bool(true),
                };
                let span = tree.expr(first).span;
                let at = self.here(span, span);
                self.check(ok, Kind::Decreases, Tag::Smaller, at, "decreases");
                before = Some(values);
            }
            self.frame.loops.push(Exits::default());
            self.block(body);
            let ways = self.frame.loops.pop().unwrap_or_default();
            exits.extend(ways.breaks);
            let mut ends = vec![(self.active, self.frame.vars.clone())];
            ends.extend(ways.continues);
            let paths: Vec<T> = ends.iter().map(|e| e.0).collect();
            self.active = self.any(&paths);
            self.frame.vars = self.select_vars(&ends);
            if self.t.as_bool(self.active) == Some(false) {
                break;
            }
        }
        let paths: Vec<T> = exits.iter().map(|e| e.0).collect();
        self.active = self.any(&paths);
        self.frame.vars = self.select_vars(&exits);
    }

    // -- expressions -------------------------------------------------------------------------

    fn cond(&mut self, e: ExprId) -> T {
        let v = self.eval(e);
        self.boolean(&v)
    }

    fn eval(&mut self, e: ExprId) -> V {
        let shape = self.shape(e);
        if self.exhausted {
            return self.default(&shape);
        }
        if self.t.len() > self.terms {
            self.exhausted = true;
            let at = self
                .module
                .functions
                .get(self.entry)
                .and_then(Option::as_ref);
            let span = at.map_or(self.span(e), |f| f.syntax.name_span);
            self.lacks(span, "a function too large to encode");
            return self.default(&shape);
        }
        let tree = self.tree();
        let span = tree.expr(e).span;
        match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => V::Int(self.t.num(Big::from_u128(*value))),
            ExprKind::Bool(b) => V::Bool(self.t.bool(*b)),
            ExprKind::Name(n) => match self.frame.vars.get(n).cloned() {
                Some(v) => v,
                None => self.default(&shape),
            },
            // A trap: the path ends here.
            ExprKind::Todo(_) | ExprKind::Unreachable(_) => {
                self.active = self.t.bool(false);
                self.default(&shape)
            }
            ExprKind::Unary { op, operand } => self.unary(e, *op, *operand),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, *lhs, *rhs),
            ExprKind::Index { base, index } => {
                let b = self.eval(*base);
                let v = self.eval(*index);
                let i = self.int(&v);
                match b {
                    V::Arr(a, Shape::Array(len, elem)) => {
                        let s = Step {
                            i,
                            key: tree.expr(*index).span,
                            trap: span,
                            len,
                        };
                        self.check_step(&s);
                        let x = self.t.select(a, i);
                        self.wrap(x, &elem)
                    }
                    // No array comes, as from a construct the encoding lacks: no path reaches
                    // the index, whose check holds and stands as the site of Tier 1's obligation.
                    V::Arr(..) | V::Int(_) | V::Bool(_) | V::Unit => {
                        let ok = self.t.bool(true);
                        let at = self.here(tree.expr(*index).span, span);
                        self.check(ok, Kind::Index, Tag::None, at, "index-out-of-bounds");
                        self.default(&shape)
                    }
                }
            }
            // An array's one field is its length, which the checker allows alone; read by its
            // name with the shape, so that another field an array may gain later is a construct
            // the encoding lacks, not a length.
            ExprKind::Field { base, name } => match (self.shape(*base), name.as_str()) {
                (Shape::Array(len, _), "len") => {
                    self.eval(*base);
                    V::Int(self.t.num(Big::from_u128(len)))
                }
                (
                    Shape::Array(..) | Shape::Int(_) | Shape::Untyped | Shape::Bool | Shape::Other,
                    _,
                ) => {
                    self.lacks(span, "a field of a struct");
                    self.default(&shape)
                }
            },
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Construct { .. } => {
                self.lacks(span, "a struct");
                V::Unit
            }
            ExprKind::Variant { .. } => {
                self.lacks(span, "an enum");
                V::Unit
            }
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, *scrutinee, arms),
            ExprKind::Array(items) => {
                let Shape::Array(_, elem) = &shape else {
                    return self.default(&shape);
                };
                let V::Arr(mut a, _) = self.default(&shape) else {
                    self.lacks(span, "a struct");
                    return V::Unit;
                };
                let func = self.frame.func;
                for (k, &item) in items.iter().enumerate() {
                    let v = self.eval(item);
                    self.narrow_plain(&v, elem, Src::of(tree, item, true), func, span);
                    if let Some(x) = term(&v) {
                        let i = self.t.num(Big::from_u128(k as u128));
                        a = self.t.store(a, i, x);
                    }
                }
                V::Arr(a, shape.clone())
            }
            ExprKind::Repeat { value, .. } => {
                let Shape::Array(_, elem) = &shape else {
                    return self.default(&shape);
                };
                let v = self.eval(*value);
                let func = self.frame.func;
                self.narrow_plain(&v, elem, Src::of(tree, *value, true), func, span);
                match term(&v) {
                    Some(x) => V::Arr(self.t.konst(x), shape.clone()),
                    None => {
                        self.lacks(span, "a struct");
                        V::Unit
                    }
                }
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let c = self.cond(*cond);
                let before = self.active;
                let saved = self.frame.vars.clone();
                self.active = self.t.and2(before, c);
                let a = self.block(*then);
                let a_end = self.active;
                let a_vars = std::mem::replace(&mut self.frame.vars, saved);
                let nc = self.t.not(c);
                self.active = self.t.and2(before, nc);
                let b = match otherwise {
                    Some(Else::Block(b)) => self.block(*b),
                    Some(Else::If(i)) => self.eval(*i),
                    None => V::Unit,
                };
                let b_end = self.active;
                let b_vars = std::mem::take(&mut self.frame.vars);
                self.active = self.t.or2(a_end, b_end);
                self.frame.vars = self.join_vars(c, a_vars, b_vars);
                self.join(c, a, b)
            }
            ExprKind::Error => self.default(&shape),
        }
    }

    fn unary(&mut self, e: ExprId, op: UnaryOp, operand: ExprId) -> V {
        let span = self.span(e);
        match op {
            UnaryOp::Not => {
                let c = self.cond(operand);
                V::Bool(self.t.not(c))
            }
            UnaryOp::Neg => {
                let v = self.eval(operand);
                let x = self.int(&v);
                let r = self.t.neg(x);
                // On an `i128` or `u128` operand, checked at the operator.
                if let Some(t) = self.shape(operand).int().filter(|t| t.bits == 128) {
                    let ok = self.in_range(r, t);
                    let at = self.here(span, span);
                    self.check(ok, Kind::Range, Tag::Into(t.name), at, "narrowing");
                }
                V::Int(r)
            }
            UnaryOp::BitNot => {
                let t = self
                    .shape(operand)
                    .int()
                    .or(self.literal(e))
                    .or(self.literal(operand))
                    .unwrap_or(I64);
                let v = self.eval(operand);
                let x = self.int(&v);
                let at_operand = self.span(operand);
                let ok = self.in_range(x, t);
                let at = self.here(at_operand, at_operand);
                self.check(ok, Kind::Range, Tag::Into(t.name), at, "narrowing");
                let u = self.t.bits(Bits::Not, t.bits, vec![x]);
                V::Int(self.signed(u, t))
            }
        }
    }

    /// The value of `t`'s two's-complement bits `u`, an unsigned value below `2^bits`.
    fn signed(&mut self, u: T, t: IntType) -> T {
        if !t.signed {
            return u;
        }
        let half = self.t.num(Big::pow2(t.bits - 1));
        let whole = self.t.num(Big::pow2(t.bits));
        let negative = self.t.le(half, u);
        let below = self.t.sub(u, whole);
        self.t.ite(negative, below, u)
    }

    fn binary(&mut self, e: ExprId, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> V {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                let a = self.cond(lhs);
                V::Bool(self.short(a, rhs, op == BinaryOp::And))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let l = self.eval(lhs);
                let r = self.eval(rhs);
                let span = self.span(e);
                let same = self.equal(&l, &r, span);
                V::Bool(if op == BinaryOp::Eq {
                    same
                } else {
                    self.t.not(same)
                })
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let l = self.eval(lhs);
                let l = self.int(&l);
                let r = self.eval(rhs);
                let r = self.int(&r);
                V::Bool(match op {
                    BinaryOp::Lt => self.t.lt(l, r),
                    BinaryOp::Le => self.t.le(l, r),
                    BinaryOp::Gt => self.t.lt(r, l),
                    BinaryOp::Ge
                    | BinaryOp::Or
                    | BinaryOp::And
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitAnd
                    | BinaryOp::Shl
                    | BinaryOp::Shr
                    | BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => self.t.le(r, l),
                })
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
                let l = self.eval(lhs);
                let l = self.int(&l);
                let r = self.eval(rhs);
                let r = self.int(&r);
                let lt = self.shape(lhs).int();
                let rt = self.shape(rhs).int();
                let taken = self.literal(e).or(self.literal(lhs));
                let work = match op {
                    BinaryOp::Shl | BinaryOp::Shr => lt.or(taken).unwrap_or(I64),
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
                    | BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => lt.or(rt).or(taken).unwrap_or(I64),
                };
                let places = Places {
                    whole: self.span(e),
                    left: self.span(lhs),
                    right: self.span(rhs),
                    one: !self.mixed(lhs),
                };
                V::Int(self.arith(op, l, r, (lt, rt), work, places))
            }
        }
    }

    /// `a and rhs` where `and`, else `a or rhs`: the right operand runs only on the path where
    /// it decides.
    fn short(&mut self, a: T, rhs: ExprId, and: bool) -> T {
        let before = self.active;
        let saved = self.frame.vars.clone();
        let go = if and { a } else { self.t.not(a) };
        self.active = self.t.and2(before, go);
        let b = self.cond(rhs);
        let ran = self.active;
        let vars = std::mem::take(&mut self.frame.vars);
        let stop = self.t.not(go);
        let skipped = self.t.and2(before, stop);
        self.active = self.t.or2(skipped, ran);
        self.frame.vars = self.join_vars(go, vars, saved);
        if and {
            self.t.and2(a, b)
        } else {
            self.t.or2(a, b)
        }
    }

    /// Whether two values are equal: integers and booleans exactly, arrays element by element.
    fn equal(&mut self, l: &V, r: &V, span: Span) -> T {
        match (l, r) {
            (V::Int(a), V::Int(b)) | (V::Bool(a), V::Bool(b)) => self.t.eq(*a, *b),
            (V::Arr(a, s), V::Arr(b, _)) => {
                let Shape::Array(len, elem) = s else {
                    return self.t.bool(false);
                };
                if s.elements().is_none() {
                    self.lacks(span, "an array of more elements than an encoding holds");
                    return self.t.bool(false);
                }
                let mut all = Vec::new();
                for k in 0..*len {
                    let i = self.t.num(Big::from_u128(k));
                    let x = self.t.select(*a, i);
                    let y = self.t.select(*b, i);
                    let (x, y) = (self.wrap(x, elem), self.wrap(y, elem));
                    all.push(self.equal(&x, &y, span));
                }
                self.t.and(&all)
            }
            (V::Int(_) | V::Bool(_) | V::Arr(..) | V::Unit, _) => {
                self.lacks(span, "a struct");
                self.t.bool(false)
            }
        }
    }

    /// An arithmetic, bit or shift operation on `l` and `r`, of the types `lt` and `rt` where
    /// they have one; `work` is the type a bit operation or a shift works in otherwise, the one
    /// its literals take.
    fn arith(
        &mut self,
        op: BinaryOp,
        l: T,
        r: T,
        (lt, rt): (Option<IntType>, Option<IntType>),
        work: IntType,
        at: Places,
    ) -> T {
        let whole = self.here(at.whole, at.whole);
        let v = match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let zero = self.t.int(0);
                let width = self.t.num(Big::from_u128(u128::from(work.bits)));
                let low = self.t.le(zero, r);
                let high = self.t.lt(r, width);
                let ok = self.t.and2(low, high);
                let at_amount = self.here(at.right, at.whole);
                self.check(ok, Kind::Shift, Tag::None, at_amount, "shift-amount");
                let v = self.shifted(op == BinaryOp::Shl, l, r, work.bits);
                let ok = self.in_range(v, work);
                let tag = if at.one {
                    Tag::Into(work.name)
                } else {
                    Tag::None
                };
                self.check(ok, Kind::Range, tag, whole, "narrowing");
                return v;
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let t = lt.or(rt).unwrap_or(work);
                let ok = self.in_range(l, t);
                let at_left = self.here(at.left, at.left);
                self.check(ok, Kind::Range, Tag::Into(t.name), at_left, "narrowing");
                let ok = self.in_range(r, t);
                let at_right = self.here(at.right, at.right);
                self.check(ok, Kind::Range, Tag::Into(t.name), at_right, "narrowing");
                let b = match op {
                    BinaryOp::BitAnd => Bits::And,
                    BinaryOp::BitOr => Bits::Or,
                    BinaryOp::BitXor
                    | BinaryOp::Or
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
                    | BinaryOp::Rem => Bits::Xor,
                };
                let u = self.t.bits(b, t.bits, vec![l, r]);
                return self.signed(u, t);
            }
            BinaryOp::Div | BinaryOp::Rem => {
                let zero = self.t.int(0);
                let is_zero = self.t.eq(r, zero);
                let ok = self.t.not(is_zero);
                let at_divisor = self.here(at.right, at.whole);
                self.check(ok, Kind::Divisor, Tag::None, at_divisor, "division-by-zero");
                // Truncated toward zero: the quotient of the magnitudes, with the sign of the
                // operands'; the remainder takes the left operand's sign.
                let lpos = self.t.le(zero, l);
                let rpos = self.t.lt(zero, r);
                let nl = self.t.neg(l);
                let nr = self.t.neg(r);
                let al = self.t.ite(lpos, l, nl);
                let ar = self.t.ite(rpos, r, nr);
                let q = self.t.div(al, ar);
                let nq = self.t.neg(q);
                let same = self.t.eq(lpos, rpos);
                let quotient = self.t.ite(same, q, nq);
                if op == BinaryOp::Div {
                    quotient
                } else {
                    let back = self.t.mul(r, quotient);
                    self.t.sub(l, back)
                }
            }
            BinaryOp::Add => self.t.add(l, r),
            BinaryOp::Sub => self.t.sub(l, r),
            BinaryOp::Mul => self.t.mul(l, r),
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => l,
        };
        // An operation on an `i128` or `u128` operand is narrowed into its type.
        if let Some(t) = [lt, rt].into_iter().flatten().find(|t| t.bits == 128) {
            let ok = self.in_range(v, t);
            self.check(ok, Kind::Range, Tag::Into(t.name), whole, "narrowing");
        }
        v
    }

    /// `l << r` where `left`, else `l >> r` rounded toward negative infinity, for each amount
    /// below `bits`.
    fn shifted(&mut self, left: bool, l: T, r: T, bits: u32) -> T {
        let mut acc = None;
        for k in (0..bits).rev() {
            let p = self.t.num(Big::pow2(k));
            let v = if left {
                self.t.mul(l, p)
            } else {
                self.t.div(l, p)
            };
            acc = Some(match acc {
                None => v,
                Some(rest) => {
                    let kk = self.t.num(Big::from_u128(u128::from(k)));
                    let here = self.t.eq(r, kk);
                    self.t.ite(here, v, rest)
                }
            });
        }
        acc.unwrap_or(l)
    }

    /// A `match` over an integer or a boolean: the scrutinee narrowed into its type, then the
    /// first arm whose pattern matches.
    fn match_expr(&mut self, e: ExprId, scrutinee: ExprId, arms: &'m [Arm]) -> V {
        let shape = self.shape(e);
        let v = self.eval(scrutinee);
        let at_scrutinee = self.span(scrutinee);
        match self.shape(scrutinee) {
            Shape::Int(t) => self.narrow_scrutinee(&v, t, at_scrutinee),
            Shape::Untyped => self.narrow_scrutinee(&v, I64, at_scrutinee),
            Shape::Bool => {}
            Shape::Array(..) | Shape::Other => {
                let span = self.span(e);
                self.lacks(span, "a match over an enum");
                return self.default(&shape);
            }
        }
        let before = self.active;
        let saved = self.frame.vars.clone();
        let mut left = self.t.bool(true);
        let mut ways: Vec<(T, (V, Vars))> = Vec::new();
        let mut ends = Vec::new();
        for arm in arms {
            let m = match &arm.pattern.kind {
                PatternKind::Wildcard => self.t.bool(true),
                PatternKind::Int {
                    negative, value, ..
                } => {
                    let x = Big::from_u128(*value);
                    let lit = self.t.num(if *negative { x.neg() } else { x });
                    let s = self.int(&v);
                    self.t.eq(s, lit)
                }
                PatternKind::Bool(b) => {
                    let lit = self.t.bool(*b);
                    let s = self.boolean(&v);
                    self.t.eq(s, lit)
                }
                PatternKind::Variant { .. } => {
                    self.lacks(arm.pattern.span, "a match over an enum");
                    self.t.bool(false)
                }
            };
            let g = self.t.and2(left, m);
            let nm = self.t.not(m);
            left = self.t.and2(left, nm);
            self.frame.vars = saved.clone();
            self.active = self.t.and2(before, g);
            let value = match arm.body {
                ArmBody::Expr(x) => self.eval(x),
                ArmBody::Block(b) => self.block(b),
            };
            ends.push(self.active);
            ways.push((g, (value, std::mem::take(&mut self.frame.vars))));
        }
        self.active = self.any(&ends);
        let vars: Vec<(T, Vars)> = ways.iter().map(|(g, (_, v))| (*g, v.clone())).collect();
        self.frame.vars = if vars.is_empty() {
            saved
        } else {
            self.select_vars(&vars)
        };
        self.select(&ways, |(v, _)| v.clone())
    }

    fn narrow_scrutinee(&mut self, v: &V, t: IntType, span: Span) {
        let x = self.int(v);
        let ok = self.in_range(x, t);
        let at = self.here(span, span);
        self.check(ok, Kind::Range, Tag::Into(t.name), at, "narrowing");
    }
}

/// Whether some path was cut off: one of `cuts`, each the condition of a path where it was cut,
/// that is not false whatever the inputs. A path cut where no input reaches it cuts nothing.
pub(crate) fn live(t: &Terms, cuts: &[T]) -> bool {
    cuts.iter().any(|&c| t.as_bool(c) != Some(false))
}

/// The term of a value, where it has one.
fn term(v: &V) -> Option<T> {
    match v {
        V::Int(x) | V::Bool(x) | V::Arr(x, _) => Some(*x),
        V::Unit => None,
    }
}

/// A frame of nothing, for a stack of calls that ran out, which a well-formed encoding never
/// does.
fn empty_frame<'m>(f: &'m Function<'m>) -> Frame<'m> {
    Frame {
        func: usize::MAX,
        f,
        vars: Vars::new(),
        declared: BTreeMap::new(),
        measure: None,
        returns: Vec::new(),
        loops: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cut cuts a path where an input may reach it: a cut of a constant false path cuts none,
    /// and one cut that may hold among such makes some path cut off.
    #[test]
    fn test_a_cut_is_live_where_an_input_may_reach_it() {
        let mut t = Terms::default();
        let (no, yes) = (t.bool(false), t.bool(true));
        let c = t.input("p.c".to_string(), Sort::Bool);
        assert!(!live(&t, &[]));
        assert!(!live(&t, &[no, no]));
        assert!(live(&t, &[no, c]));
        assert!(live(&t, &[yes]));
    }
}

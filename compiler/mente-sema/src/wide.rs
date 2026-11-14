//! Exact intermediates within 128 bits (`core-calculus.md` item 4, [`wide-intermediates`]).
//!
//! Integer arithmetic is exact, and a value is narrowed only where it lands. An operation with an
//! `i128` or `u128` operand is narrowed at the operator into that type. Every other arithmetic
//! operation has a static range, computed from its operands' ranges: a literal's is its value,
//! `.len`'s the array's length, a name's its declared refinement's when both bounds are literals,
//! an `if`'s the hull of its branches', and any other integer's its type's range. An operation
//! whose range does not lie within `u128` or within `i128` is `E0222`, whose fix binds an operand
//! to a `u128` or an `i128` first; so is an `if` whose branches' hull does not, whose fix binds
//! a branch's value to an `i128` inside that branch. The same holds wherever values meet in one
//! machine type: an operation's operands with its result, and the elements of an array literal
//! of integers of no one type. Every intermediate therefore fits one 128-bit machine type, in
//! which native code computes it without a check.
//!
//! [`wide-intermediates`]: ../../docs/design-review.md#wide-intermediates

use std::cmp::Ordering;

use mente_syntax::diagnostics::{line_col, Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, Else, ExprId, ExprKind, FnSyntax, StmtKind,
    Tree, TypeKind, UnaryOp,
};
use mente_syntax::Span;

use crate::check::Typing;
use crate::names::Resolution;
use crate::types::{Int, IntTy, Ty};

/// An integer whose magnitude lies below 2^128.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Num {
    neg: bool,
    mag: u128,
}

impl Num {
    fn new(neg: bool, mag: u128) -> Num {
        Num {
            neg: neg && mag != 0,
            mag,
        }
    }

    fn negated(self) -> Num {
        Num::new(!self.neg, self.mag)
    }

    /// The sum, or `None` when its magnitude reaches 2^128.
    fn add(self, o: Num) -> Option<Num> {
        if self.neg == o.neg {
            return Some(Num::new(self.neg, self.mag.checked_add(o.mag)?));
        }
        Some(match self.mag.cmp(&o.mag) {
            Ordering::Less => Num::new(o.neg, o.mag - self.mag),
            Ordering::Equal | Ordering::Greater => Num::new(self.neg, self.mag - o.mag),
        })
    }

    /// The product, or `None` when its magnitude reaches 2^128.
    fn mul(self, o: Num) -> Option<Num> {
        Some(Num::new(self.neg != o.neg, self.mag.checked_mul(o.mag)?))
    }
}

impl Ord for Num {
    fn cmp(&self, o: &Num) -> Ordering {
        match (self.neg, o.neg) {
            (false, false) => self.mag.cmp(&o.mag),
            (true, true) => o.mag.cmp(&self.mag),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Num {
    fn partial_cmp(&self, o: &Num) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The values an integer expression can take, as far as its operands' ranges tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Range {
    lo: Num,
    hi: Num,
}

impl Range {
    fn of(t: IntTy) -> Range {
        Range {
            lo: Num::new(true, t.min_magnitude()),
            hi: Num::new(false, t.max()),
        }
    }

    fn point(n: Num) -> Range {
        Range { lo: n, hi: n }
    }

    /// `-m ..= m`, or `0 ..= m` when the values cannot be negative.
    fn within(m: u128, non_negative: bool) -> Range {
        Range {
            lo: if non_negative {
                Num::new(false, 0)
            } else {
                Num::new(true, m)
            },
            hi: Num::new(false, m),
        }
    }

    fn magnitude(self) -> u128 {
        self.lo.mag.max(self.hi.mag)
    }

    fn hull(self, o: Range) -> Range {
        Range {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
        }
    }

    /// Whether every value lies within `u128`, or every one within `i128`.
    fn fits(self) -> bool {
        const HALF: u128 = 1 << 127;
        !self.lo.neg || (self.lo.mag <= HALF && (self.hi.neg || self.hi.mag < HALF))
    }
}

/// The arithmetic operators, whose exact results this module bounds: the others give a `bool`
/// or work on the bits of one type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl Arith {
    fn of(op: BinaryOp) -> Option<Arith> {
        match op {
            BinaryOp::Add => Some(Arith::Add),
            BinaryOp::Sub => Some(Arith::Sub),
            BinaryOp::Mul => Some(Arith::Mul),
            BinaryOp::Div => Some(Arith::Div),
            BinaryOp::Rem => Some(Arith::Rem),
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
            | BinaryOp::Shr => None,
        }
    }
}

/// Whether `a op b` can be negative.
fn may_be_negative(op: Arith, a: Range, b: Range) -> bool {
    match op {
        Arith::Sub => a.lo < b.hi,
        Arith::Add | Arith::Mul | Arith::Div | Arith::Rem => a.lo.neg || b.lo.neg,
    }
}

/// The range of `a op b`, or `None` when a bound's magnitude reaches 2^128.
fn arithmetic(op: Arith, a: Range, b: Range) -> Option<Range> {
    match op {
        Arith::Add => Some(Range {
            lo: a.lo.add(b.lo)?,
            hi: a.hi.add(b.hi)?,
        }),
        Arith::Sub => Some(Range {
            lo: a.lo.add(b.hi.negated())?,
            hi: a.hi.add(b.lo.negated())?,
        }),
        Arith::Mul => {
            let corners = [
                a.lo.mul(b.lo)?,
                a.lo.mul(b.hi)?,
                a.hi.mul(b.lo)?,
                a.hi.mul(b.hi)?,
            ];
            Some(Range {
                lo: *corners.iter().min()?,
                hi: *corners.iter().max()?,
            })
        }
        // A quotient is no larger than its dividend, and a remainder than either operand; a
        // remainder takes its dividend's sign.
        Arith::Div => Some(Range::within(a.magnitude(), !a.lo.neg && !b.lo.neg)),
        Arith::Rem => Some(Range::within(a.magnitude().min(b.magnitude()), !a.lo.neg)),
    }
}

fn int_ty(t: &Ty) -> Option<IntTy> {
    match t {
        Ty::Int(Int::One(t)) | Ty::Int(Int::Mixed(Some(t))) => Some(*t),
        Ty::Int(Int::Literal | Int::Mixed(None))
        | Ty::Bool
        | Ty::Array(..)
        | Ty::Struct(_)
        | Ty::Enum(_)
        | Ty::Unit
        | Ty::Any => None,
    }
}

/// `E0222` for each operation of `f` whose exact result might leave both `i128` and `u128`.
/// `src` is the text the spans point into, for the fixes.
pub fn check(f: &FnSyntax, resolution: &Resolution, typing: &Typing, src: &str) -> Vec<Diagnostic> {
    let tree = &f.tree;
    let mut w = Wide {
        tree,
        resolution,
        typing,
        src,
        body: match f.body {
            Some(Body::Expr(e)) => Some(e),
            _ => None,
        },
        ranges: vec![None; tree.exprs.len()],
        unfit: vec![false; tree.exprs.len()],
        errors: Vec::new(),
    };
    for i in 0..tree.exprs.len() {
        w.range(ExprId(i as u32));
    }
    for s in &tree.stmts {
        if let StmtKind::Assign {
            place,
            op: AssignOp::Compound(op),
            value,
        } = &s.kind
        {
            if let Some(op) = Arith::of(*op) {
                w.operation(s.span, Some(op), *place, Some(*value), Some(*value));
            }
        }
    }
    w.errors.sort_by_key(|d| d.primary.start);
    w.errors
}

/// A static range: the smallest and the largest value an integer expression can take, each as
/// whether it is negative and its magnitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub lo: (bool, u128),
    pub hi: (bool, u128),
}

impl From<Range> for Interval {
    fn from(r: Range) -> Interval {
        Interval {
            lo: (r.lo.neg, r.lo.mag),
            hi: (r.hi.neg, r.hi.mag),
        }
    }
}

impl From<Interval> for Range {
    fn from(i: Interval) -> Range {
        Range {
            lo: Num::new(i.lo.0, i.lo.1),
            hi: Num::new(i.hi.0, i.hi.1),
        }
    }
}

/// The static range of each expression of `f`, by its index: `None` for one that is not an
/// integer, or whose range the operations of a module that checks never need.
pub fn intervals(f: &FnSyntax, resolution: &Resolution, typing: &Typing) -> Vec<Option<Interval>> {
    let tree = &f.tree;
    let mut w = Wide {
        tree,
        resolution,
        typing,
        src: "",
        body: None,
        ranges: vec![None; tree.exprs.len()],
        unfit: vec![false; tree.exprs.len()],
        errors: Vec::new(),
    };
    (0..tree.exprs.len())
        .map(|i| w.range(ExprId(i as u32)).map(Interval::from))
        .collect()
}

/// The static range of `a op b`, for an arithmetic `op`, as a compound assignment computes it;
/// `None` when it might leave 128 bits, which `E0222` rejects, or when `op` is not arithmetic.
pub fn combine(op: BinaryOp, a: Interval, b: Interval) -> Option<Interval> {
    arithmetic(Arith::of(op)?, a.into(), b.into())
        .filter(|r| r.fits())
        .map(Interval::from)
}

struct Wide<'a> {
    tree: &'a Tree,
    resolution: &'a Resolution,
    typing: &'a Typing,
    src: &'a str,
    /// The function's body, when it is one expression.
    body: Option<ExprId>,
    /// Each expression's range once computed; `Some(None)` when it has none.
    ranges: Vec<Option<Option<Range>>>,
    /// Each array literal reported `E0222`, so that one that holds it is not reported too.
    unfit: Vec<bool>,
    errors: Vec<Diagnostic>,
}

impl Wide<'_> {
    fn typed(&self, e: ExprId) -> Option<IntTy> {
        int_ty(&self.typing.exprs[e.0 as usize])
    }

    fn range(&mut self, e: ExprId) -> Option<Range> {
        if let Some(r) = self.ranges[e.0 as usize] {
            return r;
        }
        let r = self.compute(e);
        self.ranges[e.0 as usize] = Some(r);
        r
    }

    fn compute(&mut self, e: ExprId) -> Option<Range> {
        let typed = self.typed(e).map(Range::of);
        // A bit operation, a shift or `~` over literals alone works in the type its literals
        // take, and its value lies within it (`core-calculus.md`, item 5).
        let taken = self
            .typing
            .literals
            .get(e.0 as usize)
            .copied()
            .flatten()
            .map(|t| Range::of(t.ty));
        let span = self.tree.expr(e).span;
        match &self.tree.expr(e).kind {
            ExprKind::Int { value, .. } => Some(Range::point(Num::new(false, *value))),
            ExprKind::Name(_) => self.declared(e).or(typed),
            // M1's only field is `.len`.
            ExprKind::Field { base, .. } => match &self.typing.exprs[base.0 as usize] {
                Ty::Array(n, _) => Some(Range::point(Num::new(false, *n))),
                Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => typed,
            },
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.operation(span, None, *operand, None, Some(*operand)),
            ExprKind::Binary { op, lhs, rhs } => match Arith::of(*op) {
                Some(op) => self.operation(span, Some(op), *lhs, Some(*rhs), None),
                // A comparison or a logical operator gives a `bool`, and a bit operation stays in
                // its operands' one type, or in the one their literals take.
                None => typed.or(taken),
            },
            ExprKind::Array(items) => {
                self.array(span, e, items);
                typed
            }
            ExprKind::If {
                then, otherwise, ..
            } => {
                let mut tails = vec![self.tail(*then)];
                match otherwise {
                    Some(Else::Block(b)) => tails.push(self.tail(*b)),
                    Some(Else::If(i)) => tails.push(Some(*i)),
                    None => {}
                }
                let mut hull: Option<Range> = None;
                let mut ranges = Vec::new();
                for t in tails.into_iter().flatten() {
                    let r = self.range(t)?;
                    hull = Some(hull.map_or(r, |h| h.hull(r)));
                    ranges.push((t, r));
                }
                // The branches' values meet in one machine type, so their hull must fit one. A
                // hull that fits neither holds a negative value and one above `i128`, which a
                // branch reaching above `i128` gives: bound to an `i128` in its branch, it fits.
                let top = Num::new(false, i128::MAX.unsigned_abs());
                let above = ranges.iter().find(|(_, r)| r.hi > top).map(|(t, _)| *t);
                match (hull, above) {
                    (Some(h), Some(tail)) if !h.fits() => {
                        self.report_if(span, tail);
                        None
                    }
                    _ => hull.or(typed),
                }
            }
            // A `match`'s arms meet in one machine type, as an `if`'s branches do.
            ExprKind::Match { arms, .. } => {
                let tails: Vec<(Option<ExprId>, ArmBody)> = arms
                    .iter()
                    .map(|a| match a.body {
                        ArmBody::Expr(x) => (Some(x), a.body),
                        ArmBody::Block(b) => (self.tail(b), a.body),
                    })
                    .collect();
                let mut hull: Option<Range> = None;
                let mut ranges = Vec::new();
                for (t, body) in tails {
                    let Some(t) = t else {
                        continue;
                    };
                    let r = self.range(t)?;
                    hull = Some(hull.map_or(r, |h| h.hull(r)));
                    ranges.push((t, body, r));
                }
                let top = Num::new(false, i128::MAX.unsigned_abs());
                let above = ranges.iter().find(|(_, _, r)| r.hi > top);
                match (hull, above) {
                    (Some(h), Some(&(tail, body, _))) if !h.fits() => {
                        self.report_match(span, tail, body);
                        None
                    }
                    _ => hull.or(typed),
                }
            }
            ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Repeat { .. }
            | ExprKind::Error => typed.or(taken),
        }
    }

    /// `E0222` for the `match` at `at`, whose arms' values fit no one 128-bit type together;
    /// the fix binds `tail`, the value of the arm that reaches highest, to an `i128` inside that
    /// arm, a block made of it when it is an expression, so that it is computed only when the
    /// arm runs.
    fn report_match(&mut self, at: Span, tail: ExprId, body: ArmBody) {
        let message = "the arms of this `match` give values that no one of `i128` and `u128` \
                       holds together, and they meet in one of them"
            .to_string();
        let place = self.tree.expr(tail).span;
        if let ArmBody::Block(_) = body {
            self.report_with(at, place, tail, "i128", message);
            return;
        }
        let mut d = Diagnostic::error("E0222", at, message);
        let name = self.fresh(tail);
        if let Some(value) = self.src.get(place.start..place.end) {
            let line_start = self.src[..place.start].rfind('\n').map_or(0, |i| i + 1);
            let indent: String = self.src[line_start..]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            d.fixes.push(Fix {
                message: format!("bind `{}` to `{}: i128` in its arm", value, name),
                applicability: Applicability::Suggestion,
                edits: vec![Edit {
                    span: place,
                    replacement: format!(
                        "{{\n{ind}  val {n}: i128 = {v}\n{ind}  {n}\n{ind}}}",
                        ind = indent,
                        n = name,
                        v = value
                    ),
                }],
            });
        }
        self.errors.push(d);
    }

    /// `E0222` for an array literal at `at` whose elements, integers of no one type, kept exact
    /// until the array lands, fit no one 128-bit type together. Its fix binds the element that
    /// reaches above `i128` to an `i128` first.
    fn array(&mut self, at: Span, e: ExprId, items: &[ExprId]) {
        let Ty::Array(_, elem) = &self.typing.exprs[e.0 as usize] else {
            return;
        };
        // Elements of one type are narrowed into it; only those of no standard type together
        // are kept exact, and the literals alone are points that fit. So are the integers of
        // the array literals nested in it, which are kept in the same machine type.
        let mut leaf = &**elem;
        while let Ty::Array(_, inner) = leaf {
            leaf = inner;
        }
        if !matches!(leaf, Ty::Int(Int::Mixed(None))) {
            return;
        }
        let mut ranges = Vec::new();
        if !self.leaves(items, &mut ranges) {
            return;
        }
        let hull = ranges.iter().map(|(_, r)| *r).reduce(|h, r| h.hull(r));
        let top = Num::new(false, i128::MAX.unsigned_abs());
        let above = ranges.iter().find(|(_, r)| r.hi > top).map(|(i, _)| *i);
        if let (Some(h), Some(item)) = (hull, above) {
            if !h.fits() {
                let message = "the elements of this array literal give values that no one of \
                               `i128` and `u128` holds together, and they are kept in one of them"
                    .to_string();
                self.report_with(at, at, item, "i128", message);
                self.unfit[e.0 as usize] = true;
            }
        }
    }

    /// Collects into `ranges` the range of each integer the array literal elements `items`
    /// give, through the array literals nested in them; an array of another kind gives its
    /// element type's. False where one has no range, or where a nested literal is reported
    /// `E0222` itself, so that the values are reported once, not twice.
    fn leaves(&mut self, items: &[ExprId], ranges: &mut Vec<(ExprId, Range)>) -> bool {
        for &i in items {
            // The element's own range first, which checks a nested literal on its own.
            let r = self.range(i);
            if self.unfit[i.0 as usize] {
                return false;
            }
            // An element of no type leaves before the literal holds a value
            // (`core-calculus.md`, item 6): it holds no integer, and the others are counted.
            if self.typing.exprs[i.0 as usize].has_no_value() {
                continue;
            }
            match &self.tree.expr(i).kind {
                ExprKind::Array(inner) => {
                    if !self.leaves(&inner.clone(), ranges) {
                        return false;
                    }
                }
                ExprKind::Repeat { value, .. } => {
                    if !self.leaves(&[*value], ranges) {
                        return false;
                    }
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
                | ExprKind::If { .. }
                | ExprKind::Error => {
                    let mut t = &self.typing.exprs[i.0 as usize];
                    while let Ty::Array(_, inner) = t {
                        t = inner;
                    }
                    let r = if matches!(self.typing.exprs[i.0 as usize], Ty::Array(..)) {
                        int_ty(t).map(Range::of)
                    } else {
                        r
                    };
                    let Some(r) = r else {
                        return false;
                    };
                    ranges.push((i, r));
                }
            }
        }
        true
    }

    /// A name's declared refinement, when both its bounds are literals.
    fn declared(&self, e: ExprId) -> Option<Range> {
        let b = self.resolution.names[e.0 as usize]?;
        let t = self.resolution.bindings[b.0 as usize].ty?;
        let TypeKind::Int {
            refinement: Some(r),
            ..
        } = &self.tree.ty(t).kind
        else {
            return None;
        };
        let literal = |b: &Bound| match b {
            Bound::Int { negative, value } => Some(Num::new(*negative, *value)),
            Bound::Name(_) | Bound::Len(_) => None,
        };
        let hi = literal(&r.hi)?;
        Some(Range {
            lo: literal(&r.lo)?,
            hi: if r.inclusive {
                hi
            } else {
                hi.add(Num::new(true, 1))?
            },
        })
    }

    /// The expression that gives block `b` its value, when it has one.
    fn tail(&self, b: BlockId) -> Option<ExprId> {
        match &self.tree.stmt(*self.tree.block(b).stmts.last()?).kind {
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

    /// The range of an operation at `at` on `a` and `b`, or of `-a` when `op` is `None`; `E0222`
    /// when it might leave both `i128` and `u128`. `bind` is the operand the fix binds, when the
    /// operation is not a binary one, whose larger operand it binds.
    fn operation(
        &mut self,
        at: Span,
        op: Option<Arith>,
        a: ExprId,
        b: Option<ExprId>,
        bind: Option<ExprId>,
    ) -> Option<Range> {
        let wide = [Some(a), b]
            .into_iter()
            .flatten()
            .filter_map(|o| self.typed(o))
            .find(|t| t.bits == 128);
        if let Some(t) = wide {
            return Some(Range::of(t));
        }
        let ra = self.range(a)?;
        let (exact, larger, negative, operands) = match (op, b) {
            (Some(op), Some(b)) => {
                let rb = self.range(b)?;
                let larger = if rb.magnitude() > ra.magnitude() {
                    b
                } else {
                    a
                };
                (
                    arithmetic(op, ra, rb),
                    larger,
                    may_be_negative(op, ra, rb),
                    vec![ra, rb],
                )
            }
            // A negation leaves both types only when its operand can be positive, so its result
            // can be negative.
            // A literal's negation is a constant, which reaches its type's smallest value right
            // after its `-` (`core-calculus.md` item 5): no operation, so no operand to fit.
            _ => (
                Some(Range {
                    lo: ra.hi.negated(),
                    hi: ra.lo.negated(),
                }),
                a,
                true,
                match self.tree.expr(a).kind {
                    ExprKind::Int { .. } => Vec::new(),
                    ExprKind::Bool(_)
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
                    | ExprKind::Error => vec![ra],
                },
            ),
        };
        if let Some(r) = exact.filter(|r| r.fits()) {
            // The operands meet the result in its machine type, so they must fit it too.
            let hull = operands.into_iter().fold(r, |h, o| h.hull(o));
            if hull.fits() {
                return Some(r);
            }
            // The value fits; the operand bound must hold it as the result's type does.
            let machine = if r.lo.neg { "i128" } else { "u128" };
            let src = self.src;
            let message = format!(
                "the operands of `{}` fit no one of `i128` and `u128` together with its value, \
                 and an operation computes in one of them",
                src.get(at.start..at.end).unwrap_or("")
            );
            self.report_with(at, at, bind.unwrap_or(larger), machine, message);
            return None;
        }
        // The operand is bound to the 128-bit type the operation's result needs.
        let machine = if negative { "i128" } else { "u128" };
        self.report(at, bind.unwrap_or(larger), machine);
        None
    }

    fn report(&mut self, at: Span, bind: ExprId, machine: &str) {
        let src = self.src;
        let text = |s: Span| src.get(s.start..s.end).unwrap_or("").to_string();
        let message = format!(
            "the exact value of `{}` might need more than 128 bits, and an intermediate is \
             computed exactly only within `i128` or `u128`",
            text(at)
        );
        self.report_with(at, at, bind, machine, message);
    }

    /// `E0222` for the `if` at `at`, whose branches' values fit no one 128-bit type together;
    /// the fix binds `tail`, the value of the branch that reaches highest, to an `i128` inside
    /// that branch, so that it is computed only when the branch is taken.
    fn report_if(&mut self, at: Span, tail: ExprId) {
        let message = "the branches of this `if` give values that no one of `i128` and `u128` \
                       holds together, and they meet in one of them"
            .to_string();
        let place = self.tree.expr(tail).span;
        self.report_with(at, place, tail, "i128", message);
    }

    /// `E0222` at `at`, whose fix binds `bind` to `machine` before the statement that holds
    /// `place`.
    fn report_with(&mut self, at: Span, place: Span, bind: ExprId, machine: &str, message: String) {
        let src = self.src;
        let text = |s: Span| src.get(s.start..s.end).unwrap_or("").to_string();
        let operand = self.tree.expr(bind).span;
        let mut d = Diagnostic::error("E0222", at, message);
        let name = self.fresh(bind);
        let line = format!("val {}: {} = {}", name, machine, text(operand));
        // An arm of a `match` alone computes what it holds, so a binding for it goes into
        // the arm, made a block, as a branch of an `if` keeps it in its block.
        let edits = match self.arm_around(place) {
            Some(body) => self.arm_block(body, operand, &line, &name).map(|e| vec![e]),
            None => match self.insertion(place, &line) {
                Some(insert) => Some(vec![
                    insert,
                    Edit {
                        span: operand,
                        replacement: name.clone(),
                    },
                ]),
                None => self.block_body(at, operand, &line, &name).map(|e| vec![e]),
            },
        };
        if let Some(edits) = edits {
            d.fixes.push(Fix {
                message: format!("bind `{}` to `{}: {}` first", text(operand), name, machine),
                applicability: Applicability::Suggestion,
                edits,
            });
        }
        self.errors.push(d);
    }

    /// For an operation in a function's expression body, the edit that turns `= body` into a
    /// block that declares `line` and then gives the body with the operand replaced by `name`.
    fn block_body(&self, at: Span, operand: Span, line: &str, name: &str) -> Option<Edit> {
        // An operation outside the body lies before it, in a contract clause.
        let body = self.tree.expr(self.body?).span;
        if at.start < body.start {
            return None;
        }
        let equals = self.src.get(..body.start)?.rfind('=')?;
        let value = format!(
            "{}{}{}",
            self.src.get(body.start..operand.start)?,
            name,
            self.src.get(operand.end..body.end)?
        );
        let (l, c) = line_col(self.src, equals);
        Some(Edit {
            span: Span {
                start: equals,
                end: body.end,
                line: l as u32,
                col: c as u32,
            },
            replacement: format!(
                "{{
  {}
  {}
}}",
                line, value
            ),
        })
    }

    /// A name for the binding the fix declares, which no binding of the function has.
    fn fresh(&self, e: ExprId) -> String {
        let base = match &self.tree.expr(e).kind {
            ExprKind::Binary {
                op: BinaryOp::Mul, ..
            } => "product",
            ExprKind::Binary {
                op: BinaryOp::Add, ..
            } => "sum",
            ExprKind::Binary {
                op: BinaryOp::Sub, ..
            } => "difference",
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
            | ExprKind::Error => "wide",
        };
        let taken = |n: &str| self.resolution.bindings.iter().any(|b| b.name == n);
        (1..)
            .map(|k| match k {
                1 => base.to_string(),
                k => format!("{}{}", base, k),
            })
            .find(|n| !taken(n))
            .unwrap_or_default()
    }

    /// The edit that inserts `line` before the statement that holds `at`: none for a loop's
    /// condition, which runs every round, and outside the body.
    /// The body of the innermost arm of a `match` that holds `at` and is an expression, not a
    /// block, when no statement inside it holds `at`: a binding `at` needs goes into it.
    fn arm_around(&self, at: Span) -> Option<Span> {
        let inside = |s: Span| s.start <= at.start && at.end <= s.end;
        let body = self
            .tree
            .exprs
            .iter()
            .filter_map(|x| match &x.kind {
                ExprKind::Match { arms, .. } => Some(arms),
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
                | ExprKind::Array(_)
                | ExprKind::Repeat { .. }
                | ExprKind::If { .. }
                | ExprKind::Error => None,
            })
            .flatten()
            .filter_map(|a| match a.body {
                ArmBody::Expr(x) => Some(self.tree.expr(x).span),
                ArmBody::Block(_) => None,
            })
            .filter(|s| inside(*s))
            .max_by_key(|s| s.start)?;
        // A statement that holds `at` either holds the whole `match` or starts inside the arm.
        let held = self
            .tree
            .stmts
            .iter()
            .any(|s| inside(s.span) && body.start <= s.span.start);
        (!held).then_some(body)
    }

    /// The edit that makes the arm `body` a block declaring `line`, then giving the arm's
    /// value with `operand` replaced by `name`.
    fn arm_block(&self, body: Span, operand: Span, line: &str, name: &str) -> Option<Edit> {
        let text = self.src.get(body.start..body.end)?;
        let from = operand.start.checked_sub(body.start)?;
        let to = operand.end.checked_sub(body.start)?;
        let value = format!("{}{}{}", text.get(..from)?, name, text.get(to..)?);
        let line_start = self.src.get(..body.start)?.rfind('\n').map_or(0, |i| i + 1);
        let indent: String = self
            .src
            .get(line_start..)?
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        Some(Edit {
            span: body,
            replacement: format!(
                "{{\n{ind}  {line}\n{ind}  {value}\n{ind}}}",
                ind = indent,
                line = line,
                value = value
            ),
        })
    }

    fn insertion(&self, at: Span, line: &str) -> Option<Edit> {
        let stmt = self
            .tree
            .stmts
            .iter()
            .filter(|s| s.span.start <= at.start && at.end <= s.span.end)
            .max_by_key(|s| s.span.start)
            .filter(|s| !matches!(s.kind, StmtKind::While { .. }))?;
        let start = stmt.span.start;
        let line_start = self.src.get(..start)?.rfind('\n').map_or(0, |i| i + 1);
        let lead = self.src.get(line_start..start)?;
        let point = |byte: usize| {
            let (line, col) = line_col(self.src, byte);
            Span {
                start: byte,
                end: byte,
                line: line as u32,
                col: col as u32,
            }
        };
        Some(if lead.trim().is_empty() {
            Edit {
                span: point(line_start),
                replacement: format!("{}{}\n", lead, line),
            }
        } else {
            let indent: String = lead
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            Edit {
                span: point(start),
                replacement: format!("{}\n{}", line, indent),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use mente_syntax::diagnostics::apply_fix;

    use super::{arithmetic, may_be_negative, Arith, Num, Range};
    use crate::check_source;

    fn n(v: i128) -> Num {
        Num::new(v < 0, v.unsigned_abs())
    }

    fn r(lo: i128, hi: i128) -> Range {
        Range {
            lo: n(lo),
            hi: n(hi),
        }
    }

    #[test]
    fn test_ranges_follow_interval_arithmetic() {
        assert_eq!(n(5).add(n(-7)), Some(n(-2)));
        assert_eq!(n(-5).add(n(7)), Some(n(2)));
        assert_eq!(n(-5).add(n(-7)), Some(n(-12)));
        assert_eq!(Num::new(false, u128::MAX).add(n(1)), None);
        assert_eq!(n(-3).mul(n(4)), Some(n(-12)));
        assert_eq!(Num::new(false, 1 << 64).mul(Num::new(false, 1 << 64)), None);
        assert_eq!(arithmetic(Arith::Add, r(-1, 2), r(3, 4)), Some(r(2, 6)));
        assert_eq!(arithmetic(Arith::Sub, r(0, 10), r(1, 3)), Some(r(-3, 9)));
        assert_eq!(arithmetic(Arith::Mul, r(-2, 3), r(4, 5)), Some(r(-10, 15)));
        // A quotient is no larger than its dividend, and negative only with a negative operand.
        assert_eq!(arithmetic(Arith::Div, r(0, 9), r(1, 3)), Some(r(0, 9)));
        assert_eq!(arithmetic(Arith::Div, r(0, 9), r(-3, 3)), Some(r(-9, 9)));
        assert_eq!(arithmetic(Arith::Div, r(-9, 2), r(1, 3)), Some(r(-9, 9)));
        // A remainder is no larger than either operand, and takes its dividend's sign.
        assert_eq!(arithmetic(Arith::Rem, r(0, 9), r(-4, 4)), Some(r(0, 4)));
        assert_eq!(
            arithmetic(Arith::Rem, r(-20, 9), r(1, 30)),
            Some(r(-20, 20))
        );
    }

    #[test]
    fn test_a_range_fits_when_one_128_bit_type_holds_it() {
        let top = Num::new(false, u128::MAX);
        assert!(Range { lo: n(0), hi: top }.fits());
        assert!(r(i128::MIN, i128::MAX).fits());
        assert!(!Range {
            lo: n(-1),
            hi: Num::new(false, 1 << 127)
        }
        .fits());
        assert!(!Range {
            lo: Num::new(true, (1 << 127) + 1),
            hi: n(0)
        }
        .fits());
        assert!(!Range { lo: n(-1), hi: top }.fits());
    }

    #[test]
    fn test_a_result_may_be_negative_by_its_operands_signs() {
        assert!(!may_be_negative(Arith::Sub, r(3, 10), r(0, 3)));
        assert!(may_be_negative(Arith::Sub, r(2, 10), r(0, 3)));
        assert!(!may_be_negative(Arith::Mul, r(0, 1), r(0, 1)));
        assert!(may_be_negative(Arith::Mul, r(-1, 1), r(0, 1)));
        assert!(may_be_negative(Arith::Mul, r(0, 1), r(-1, 1)));
    }

    fn module(src: &str) -> String {
        format!("module t\n{}\n", src)
    }

    fn codes(src: &str) -> Vec<&'static str> {
        check_source(&module(src))
            .err()
            .unwrap_or_default()
            .iter()
            .map(|d| d.code)
            .collect()
    }

    /// The module after the fix of its only diagnostic, an `E0222` with the fix `message`.
    fn fixed(src: &str, message: &str) -> String {
        let text = module(src);
        let d = check_source(&text).err().unwrap_or_default();
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(d[0].code, "E0222");
        assert_eq!(d[0].fixes[0].message, message);
        apply_fix(&text, &d[0].fixes[0]).unwrap()
    }

    #[test]
    fn test_an_operation_whose_range_leaves_128_bits_is_e0222() {
        let cube = "pub fn cube(a: u64) -> u128 {\n  val c: u128 = a * a * a\n  c\n}";
        let d = check_source(&module(cube)).unwrap_err();
        assert_eq!(
            d[0].message,
            "the exact value of `a * a * a` might need more than 128 bits, and an intermediate is computed exactly only within `i128` or `u128`"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (3, 17));
        let after = fixed(cube, "bind `a * a` to `product: u128` first");
        assert!(
            after.contains("  val product: u128 = a * a\n  val c: u128 = product * a\n"),
            "{}",
            after
        );
        assert!(check_source(&after).is_ok(), "{}", after);
        // A sum and a difference of products leave 128 bits too; a difference may be negative.
        assert_eq!(
            codes("pub fn f(a: u64, b: u64) -> u128 = a * b + a * b * 0 + a * b"),
            ["E0222"]
        );
        let diff = fixed(
            "pub fn f(a: i64, b: i64, c: i64) -> i128 {\n  val d: i128 = a * b * c\n  d\n}",
            "bind `a * b` to `product: i128` first",
        );
        assert!(check_source(&diff).is_ok(), "{}", diff);
    }

    /// A `match`'s arms meet in one machine type as an `if`'s branches do: arms whose values no
    /// one 128-bit type holds together are `E0222`, whose fix binds the value of the arm that
    /// reaches highest to an `i128` inside that arm, a block made of it where it was an
    /// expression; and the module then checks.
    #[test]
    fn test_a_match_whose_arms_leave_128_bits_together_is_e0222() {
        let mixed =
            "pub fn f(c: bool, a: i64, b: u128) -> i64 = match c {\n  true => a\n  false => b\n}";
        assert_eq!(codes(mixed), ["E0222"]);
        let text = fixed(mixed, "bind `b` to `wide: i128` in its arm");
        assert!(
            text.contains("  false => {\n    val wide: i128 = b\n    wide\n  }"),
            "{}",
            text
        );
        assert_eq!(check_source(&text).err(), None, "{}", text);
        let block = "pub fn f(c: bool, a: i64, b: u128) -> i64 = match c {\n  true => a\n  false => {\n    b\n  }\n}";
        let text = fixed(block, "bind `b` to `wide: i128` first");
        assert_eq!(check_source(&text).err(), None, "{}", text);
        // An operation in an arm that is an expression is bound in that arm, made a block,
        // where the pattern's names are visible and the arm alone computes it; the fixed module
        // checks.
        let arm = "pub enum E {\n  A\n  B(n: u64)\n}\npub fn f(e: E) -> u128 = match e {\n  E.B(n: n) => n * n * n\n  E.A => 0\n}";
        let text = fixed(arm, "bind `n * n` to `product: u128` first");
        assert!(
            text.contains("  E.B(n: n) => {\n    val product: u128 = n * n\n    product * n\n  }"),
            "{}",
            text
        );
        assert_eq!(check_source(&text).err(), None, "{}", text);
        // So in a statement's arm, where the element it reads, bound before the `match`, would
        // be read though the arm never runs.
        let stmt = "pub fn f(c: bool, xs: [2]u64, i: usize) -> u128 {\n  val r: u128 = match c {\n    true => xs[i] * xs[i] * xs[i]\n    false => 0\n  }\n  r\n}";
        let text = fixed(stmt, "bind `xs[i] * xs[i]` to `product: u128` first");
        assert!(
            text.contains(
                "    true => {\n      val product: u128 = xs[i] * xs[i]\n      product * xs[i]\n    }"
            ),
            "{}",
            text
        );
        assert_eq!(check_source(&text).err(), None, "{}", text);
        // A statement inside the arm that holds the operation takes the binding, as anywhere.
        let held = "pub enum E {\n  A\n  B(n: u64)\n}\npub fn f(e: E, c: bool) -> u128 = match e {\n  E.B(n: n) => if c {\n    val k: u128 = n * n * n\n    k\n  } else {\n    0\n  }\n  E.A => 0\n}";
        let text = fixed(held, "bind `n * n` to `product: u128` first");
        assert!(
            text.contains("{\n    val product: u128 = n * n\n    val k: u128 = product * n\n"),
            "{}",
            text
        );
        assert_eq!(check_source(&text).err(), None, "{}", text);
        // The arm bound is the one above `i128`, never one whose top is `i128`'s.
        let top =
            "pub fn f(c: bool, a: i128, b: u128) -> i128 = match c {\n  true => a\n  false => b\n}";
        let text = fixed(top, "bind `b` to `wide: i128` in its arm");
        assert_eq!(check_source(&text).err(), None, "{}", text);
        // Arms that fit one type together are none.
        for src in [
            "pub fn f(c: bool, a: i64, b: u64) -> i64 = match c {\n  true => a\n  false => b\n}",
            "pub fn f(c: bool, a: u8, b: u128) -> u8 = match c {\n  true => a\n  false => b\n}",
        ] {
            assert!(codes(src).is_empty(), "{}", src);
        }
    }

    #[test]
    fn test_an_if_whose_branches_leave_128_bits_together_is_e0222() {
        // An `i64` and a `u128`: no one 128-bit type holds both, where the branches meet.
        let mixed = "pub fn f(c: bool, a: i64, b: u128) -> i64 {\n  if c {\n    a\n  } else {\n    b\n  }\n}";
        let d = check_source(&module(mixed)).unwrap_err();
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(
            d[0].message,
            "the branches of this `if` give values that no one of `i128` and `u128` holds together, and they meet in one of them"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (3, 3));
        // The fix binds the highest branch's value inside its branch, where it alone is computed.
        let after = fixed(mixed, "bind `b` to `wide: i128` first");
        assert!(
            after.contains("  } else {\n    val wide: i128 = b\n    wide\n  }"),
            "{}",
            after
        );
        assert!(check_source(&after).is_ok(), "{}", after);
        // The fix binds the branch that reaches above `i128`, never one whose top is `i128`'s.
        let top = "pub fn f(c: bool, a: i128, b: u128) -> i8 {
  if c {
    a
  } else {
    b
  }
}";
        let after = fixed(top, "bind `b` to `wide: i128` first");
        assert!(check_source(&after).is_ok(), "{}", after);
        // An `else if` chain is judged as a whole.
        assert_eq!(
            codes("pub fn f(c: bool, d: bool, a: i8, b: u8, e: u128) -> i8 {\n  if c {\n    a\n  } else if d {\n    b\n  } else {\n    e\n  }\n}"),
            ["E0222"]
        );
        // Branches whose hull fits one type are fine, signed or not.
        for ok in [
            "pub fn f(c: bool, a: i64, b: u64) -> i64 = if c { a } else { b }",
            "pub fn f(c: bool, a: u8, b: u128) -> u8 = if c { a } else { b }",
            "pub fn f(c: bool, a: i128, b: u64) -> i8 = if c { a } else { b }",
        ] {
            assert!(codes(ok).is_empty(), "{}: {:?}", ok, codes(ok));
        }
    }

    #[test]
    fn test_values_that_meet_in_one_machine_type_fit_one_together() {
        // An operation's operands meet its value: a negative one beside one above `i128`.
        let ops = "pub fn s(a: u8, b: i8) -> u128 {\n  val v: u128 = b + (a + 170141183460469231731687303715884105727)\n  v\n}";
        let d = check_source(&module(ops)).unwrap_err();
        assert_eq!(
            d[0].message,
            "the operands of `b + (a + 170141183460469231731687303715884105727)` fit no one of `i128` and `u128` together with its value, and an operation computes in one of them"
        );
        // Bound to a `u128`, which holds the never-negative value, the operand makes the
        // operation one on a `u128`, checked at the operator.
        let after = fixed(
            ops,
            "bind `a + 170141183460469231731687303715884105727` to `sum: u128` first",
        );
        assert!(check_source(&after).is_ok(), "{}", after);
        assert!(codes("pub fn s(a: u8, b: i8) -> u128 = b + (a + 100)").is_empty());
        // A literal's negation is a constant, `i128`'s smallest value included.
        assert!(codes("pub fn m() -> i128 = -170141183460469231731687303715884105728").is_empty());
        // An array literal of integers of no one type keeps its elements in one machine type.
        let arr = "pub fn r(a: i32, b: u128) -> [2]i128 {\n  val xs: [2]i128 = [a, b]\n  xs\n}";
        let d = check_source(&module(arr)).unwrap_err();
        assert_eq!(
            d[0].message,
            "the elements of this array literal give values that no one of `i128` and `u128` holds together, and they are kept in one of them"
        );
        let after = fixed(arr, "bind `b` to `wide: i128` first");
        assert!(check_source(&after).is_ok(), "{}", after);
        // The element bound is the one above `i128`, never one whose top is `i128`'s.
        let top = "pub fn r(a: i128, b: u128) -> [2]i128 {\n  val xs: [2]i128 = [a, b]\n  xs\n}";
        let after = fixed(top, "bind `b` to `wide: i128` first");
        assert!(check_source(&after).is_ok(), "{}", after);
        // So does an array literal of array literals, for the integers of every one nested in
        // it: those that fit no one type together are `E0222` once, at the literal that holds
        // them all, and the fix applies.
        let nested =
            "pub fn r(a: i32, b: u128) -> [2][1]i128 {\n  val xs: [2][1]i128 = [[a], [b]]\n  xs\n}";
        assert_eq!(codes(nested), ["E0222"]);
        let after = fixed(nested, "bind `b` to `wide: i128` first");
        assert!(check_source(&after).is_ok(), "{}", after);
        let inner =
            "pub fn r(a: i32, b: u128) -> [1][2]i128 {\n  val xs: [1][2]i128 = [[a, b]]\n  xs\n}";
        assert_eq!(codes(inner), ["E0222"]);
        let repeated =
            "pub fn r(a: i32, b: u128) -> [2][2]i128 {\n  val xs: [2][2]i128 = [[a, 0], [b; 2]]\n  xs\n}";
        assert_eq!(codes(repeated), ["E0222"]);
        for ok in [
            "pub fn r(a: i32, b: u64) -> [2]i128 = [a, b]",
            "pub fn r(a: u64, b: u128) -> [2]u128 = [a, b]",
            "pub fn r(a: i8, b: i8) -> [2]i8 = [a, b]",
            "pub fn r(a: i32, b: u64) -> [2][1]i128 = [[a], [b]]",
            "pub fn r(a: u64, b: u128) -> [2][1]u128 = [[a], [b]]",
            // A repeated value's range, not its type's: `a` lies within 0 to 3.
            "pub fn r(a: i32[0..4], b: u128) -> [2][1]u128 = [[b], [a; 1]]",
            // An array among them that is no literal counts with its element type's range.
            "pub fn r(p: [1]i8, b: u128[35..=45]) -> [2][1]i8 = [[b], p]",
            "pub fn r(p: [1]i8, b: u128[35..=45]) -> [2][1]i8 = [p, [b]]",
            "pub fn r(a: i64, b: u64) -> [3]i128 = [a, b, todo(\"x\")]",
        ] {
            assert!(codes(ok).is_empty(), "{}: {:?}", ok, codes(ok));
        }
        // Its element type's whole range, whatever it holds: an `i8` may be negative, and a
        // `u128` of no refinement reaches above `i128`.
        for unfit in [
            "pub fn r(p: [1]i8, b: u128) -> [2][1]i128 = [[b], p]",
            "pub fn r(p: [1]i8, b: u128) -> [2][1]i128 = [p, [b]]",
            // An element of no type holds no integer: the others are counted all the same.
            "pub fn r(a: i128, b: u128) -> [3]i128 = [a, b, todo(\"x\")]",
            "pub fn r(a: i128, b: u128) -> [3][1]i128 = [[a], [b], [todo(\"x\")]]",
            "pub fn r(a: i128, b: u128, c: bool) -> [3]i128 = [a, if c { return [0; 3] } else { return [1; 3] }, b]",
        ] {
            assert_eq!(codes(unfit), ["E0222"], "{}", unfit);
        }
    }

    #[test]
    fn test_ranges_come_from_literals_refinements_and_lengths() {
        for ok in [
            "pub fn f(a: u64[0..1000], b: u64) -> u128 = a * a * b",
            "pub fn f(a: u64[0..=999], b: u64) -> u128 = a * a * b",
            "pub fn f(a: u64, b: u64) -> u128 = a * b * 1",
            "pub fn f(xs: [1]u8, a: u64, b: u64) -> u128 = xs.len * a * b",
            "pub fn f(a: u64, b: u64) -> u128 = a * b + a",
            "pub fn f(a: u64, b: u64, c: u64) -> u128 = a * b / c + a",
            "pub fn f(a: u64, b: u64, c: u64) -> u128 = a * b % c * c",
            "pub fn f(a: i64, b: i64) -> i128 = -(a * b) + 1",
        ] {
            assert!(codes(ok).is_empty(), "{}: {:?}", ok, codes(ok));
        }
        for wide in [
            "pub fn f(a: u64, b: u64) -> u128 = a * b * 2",
            "pub fn f(a: u64[0..1000], b: u64) -> u128 = b * b * a",
            "pub fn f(a: u64[0..n], n: u64, b: u64) -> u128 = b * b * a",
            "pub fn f(xs: [4]u64, b: u64) -> u128 = xs[0] * b * b",
            "pub fn f(c: bool, a: u64, b: u64) -> u128 = (if c { a * b } else { 0 }) * a",
            "pub fn f(c: bool, a: u64, b: u64) -> u128 = (if c { 0 } else { a * b }) * a",
            "pub fn f(a: u64, b: u64, c: u64) -> u128 = a * b * c / c",
            // A shift narrows into its operand's type, whatever the amount.
            "pub fn f(a: u64, b: u64) -> u128 = (a << 1) * b * b",
        ] {
            assert_eq!(codes(wide), ["E0222"], "{}", wide);
        }
    }

    #[test]
    fn test_128_bit_operands_are_checked_at_the_operator_instead() {
        assert!(codes("pub fn f(x: u128, y: u64) -> u128 = x * y * y").is_empty());
        assert!(codes("pub fn f(x: i128, y: i128) -> i128 = -(x * y) - x").is_empty());
    }

    #[test]
    fn test_negations_and_compound_assignments_are_operations() {
        let neg = fixed(
            "pub fn f(a: u64, b: u64) -> i128 = -(a * b)",
            "bind `a * b` to `product: i128` first",
        );
        assert!(
            neg.contains("-> i128 {\n  val product: i128 = a * b\n  -(product)\n}"),
            "{}",
            neg
        );
        assert!(check_source(&neg).is_ok(), "{}", neg);
        let compound = fixed(
            "pub fn f(a: u64, b: u64) -> u64 {\n  var x: u64 = a\n  x *= a * b\n  x\n}",
            "bind `a * b` to `product: u128` first",
        );
        assert!(check_source(&compound).is_ok(), "{}", compound);
        assert!(
            codes("pub fn f(a: u64, b: u64) -> u64 {\n  var x: u64 = a\n  x += a * b\n  x\n}")
                .is_empty()
        );
    }

    #[test]
    fn test_the_fix_goes_before_the_statement_under_a_fresh_name() {
        // A statement that shares its line keeps the line's indentation after the binding.
        let inline = fixed(
            "pub fn f(c: bool, product: u64, a: u64) -> u128 {\n  if c { return product * a * a }\n  0\n}",
            "bind `product * a` to `product2: u128` first",
        );
        assert!(
            inline.contains("  if c { val product2: u128 = product * a\n  return product2 * a }"),
            "{}",
            inline
        );
        let sum = fixed(
            "pub fn f(a: u128, b: u64, c: u64) -> u128 {\n  val s: u128 = (b * c + b) * b * c\n  s\n}",
            "bind `b * c + b` to `sum: u128` first",
        );
        assert!(check_source(&sum).is_ok(), "{}", sum);
        // Of operands of one range, the left is bound; a difference is named so.
        let tie = fixed(
            "pub fn f(a: u64, b: u64, c: u64, d: u64) -> u128 {\n  val x: u128 = a * b * (c * d)\n  x\n}",
            "bind `a * b` to `product: u128` first",
        );
        assert!(check_source(&tie).is_ok(), "{}", tie);
        let difference = fixed(
            "pub fn f(a: i64, b: i64, c: i64, d: i64) -> i128 {\n  val x: i128 = (a * b - c) * d\n  x\n}",
            "bind `a * b - c` to `difference: i128` first",
        );
        assert!(check_source(&difference).is_ok(), "{}", difference);
        // Inside an expression body, the body becomes a block.
        let inner = fixed(
            "pub fn f(a: u64, b: u64) -> u128 = 1 + a * b * 2",
            "bind `a * b` to `product: u128` first",
        );
        assert!(
            inner.contains(
                "-> u128 {
  val product: u128 = a * b
  1 + product * 2
}"
            ),
            "{}",
            inner
        );
        assert!(check_source(&inner).is_ok(), "{}", inner);
        // In a contract clause, which no statement holds, no binding is offered.
        let contract = check_source(&module(
            "pub fn f(a: u64) -> u128\n  ensures result <= a * a * a\n= a",
        ))
        .unwrap_err();
        assert_eq!(contract[0].code, "E0222");
        assert!(contract[0].fixes.is_empty());
        // In a loop's condition, which runs every round, no binding is offered.
        let d = check_source(&module(
            "pub fn f(a: u64, n: u128) -> u64 {\n  var k: u64 = 0\n  while a * a * a > n {\n    k += 1\n  }\n  k\n}",
        ))
        .unwrap_err();
        assert_eq!(d[0].code, "E0222");
        assert!(d[0].fixes.is_empty());
    }
}

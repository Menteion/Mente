//! Exact intermediates within 128 bits (`core-calculus.md` item 4, [`wide-intermediates`]).
//!
//! Integer arithmetic is exact, and a value is narrowed only where it lands. An operation with an
//! `i128` or `u128` operand is narrowed at the operator into that type. Every other arithmetic
//! operation has a static range, computed from its operands' ranges: a literal's is its value,
//! `.len`'s the array's length, a name's its declared refinement's when both bounds are literals,
//! an `if`'s the hull of its branches', and any other integer's its type's range. An operation
//! whose range does not lie within `u128` or within `i128` is `E0222`, whose fix binds an operand
//! to a `u128` or an `i128` first. Every intermediate therefore fits one 128-bit machine type,
//! in which native code computes it without a check.
//!
//! [`wide-intermediates`]: ../../docs/design-review.md#wide-intermediates

use std::cmp::Ordering;

use mente_syntax::diagnostics::{line_col, Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    AssignOp, BinaryOp, BlockId, Body, Bound, Else, ExprId, ExprKind, FnSyntax, StmtKind, Tree,
    TypeKind, UnaryOp,
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
            _ => Num::new(self.neg, self.mag - o.mag),
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
            lo: Num::new(!non_negative, m),
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

/// Whether `a op b` can be negative.
fn may_be_negative(op: BinaryOp, a: Range, b: Range) -> bool {
    match op {
        BinaryOp::Sub => a.lo < b.hi,
        _ => a.lo.neg || b.lo.neg,
    }
}

/// The range of `a op b`, or `None` when a bound's magnitude reaches 2^128.
fn arithmetic(op: BinaryOp, a: Range, b: Range) -> Option<Range> {
    match op {
        BinaryOp::Add => Some(Range {
            lo: a.lo.add(b.lo)?,
            hi: a.hi.add(b.hi)?,
        }),
        BinaryOp::Sub => Some(Range {
            lo: a.lo.add(b.hi.negated())?,
            hi: a.hi.add(b.lo.negated())?,
        }),
        BinaryOp::Mul => {
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
        BinaryOp::Div => Some(Range::within(a.magnitude(), !a.lo.neg && !b.lo.neg)),
        _ => Some(Range::within(a.magnitude().min(b.magnitude()), !a.lo.neg)),
    }
}

fn is_arithmetic(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
    )
}

fn int_ty(t: &Ty) -> Option<IntTy> {
    match t {
        Ty::Int(Int::One(t)) | Ty::Int(Int::Mixed(Some(t))) => Some(*t),
        _ => None,
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
            if is_arithmetic(*op) {
                w.operation(s.span, Some(*op), *place, Some(*value), Some(*value));
            }
        }
    }
    w.errors.sort_by_key(|d| d.primary.start);
    w.errors
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
        let span = self.tree.expr(e).span;
        match &self.tree.expr(e).kind {
            ExprKind::Int { value, .. } => Some(Range::point(Num::new(false, *value))),
            ExprKind::Name(_) => self.declared(e).or(typed),
            ExprKind::Field { base, name } if name == "len" => {
                match &self.typing.exprs[base.0 as usize] {
                    Ty::Array(n, _) => Some(Range::point(Num::new(false, *n))),
                    _ => typed,
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.operation(span, None, *operand, None, Some(*operand)),
            ExprKind::Binary { op, lhs, rhs } if is_arithmetic(*op) => {
                self.operation(span, Some(*op), *lhs, Some(*rhs), None)
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
                for t in tails.into_iter().flatten() {
                    let r = self.range(t)?;
                    hull = Some(hull.map_or(r, |h| h.hull(r)));
                }
                hull.or(typed)
            }
            _ => typed,
        }
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
            _ => None,
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
            _ => None,
        }
    }

    /// The range of an operation at `at` on `a` and `b`, or of `-a` when `op` is `None`; `E0222`
    /// when it might leave both `i128` and `u128`. `bind` is the operand the fix binds, when the
    /// operation is not a binary one, whose larger operand it binds.
    fn operation(
        &mut self,
        at: Span,
        op: Option<BinaryOp>,
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
        let (exact, larger, negative) = match (op, b) {
            (Some(op), Some(b)) => {
                let rb = self.range(b)?;
                let larger = if rb.magnitude() > ra.magnitude() {
                    b
                } else {
                    a
                };
                (arithmetic(op, ra, rb), larger, may_be_negative(op, ra, rb))
            }
            _ => (
                Some(Range {
                    lo: ra.hi.negated(),
                    hi: ra.lo.negated(),
                }),
                a,
                !ra.hi.neg && ra.hi.mag > 0,
            ),
        };
        if let Some(r) = exact.filter(|r| r.fits()) {
            return Some(r);
        }
        // The operand is bound to the 128-bit type the operation's result needs.
        let machine = if negative { "i128" } else { "u128" };
        self.report(at, bind.unwrap_or(larger), machine);
        None
    }

    fn report(&mut self, at: Span, bind: ExprId, machine: &str) {
        let src = self.src;
        let text = |s: Span| src.get(s.start..s.end).unwrap_or("").to_string();
        let operand = self.tree.expr(bind).span;
        let mut d = Diagnostic::error(
            "E0222",
            at,
            format!(
                "the exact value of `{}` might need more than 128 bits, and an intermediate is \
                 computed exactly only within `i128` or `u128`",
                text(at)
            ),
        );
        let name = self.fresh(bind);
        let line = format!("val {}: {} = {}", name, machine, text(operand));
        let edits = match self.insertion(at, &line) {
            Some(insert) => Some(vec![
                insert,
                Edit {
                    span: operand,
                    replacement: name.clone(),
                },
            ]),
            None => self.block_body(at, operand, &line, &name).map(|e| vec![e]),
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
        let body = self.tree.expr(self.body?).span;
        if at.start < body.start || body.end < at.end {
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
            _ => "wide",
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
    fn insertion(&self, at: Span, line: &str) -> Option<Edit> {
        let stmt = self
            .tree
            .stmts
            .iter()
            .filter(|s| s.span.start <= at.start && at.end <= s.span.end)
            .min_by_key(|s| s.span.end - s.span.start)
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

    use crate::check_source;

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
        // In a loop's condition, which runs every round, no binding is offered.
        let d = check_source(&module(
            "pub fn f(a: u64, n: u128) -> u64 {\n  var k: u64 = 0\n  while a * a * a > n {\n    k += 1\n  }\n  k\n}",
        ))
        .unwrap_err();
        assert_eq!(d[0].code, "E0222");
        assert!(d[0].fixes.is_empty());
    }
}

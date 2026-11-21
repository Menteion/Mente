//! What decides or states changes nothing (`core-calculus.md`, item 10;
//! [`pure-conditions`](../../docs/design-review.md#pure-conditions)).
//!
//! A condition, in this rule, is each place whose evaluation decides what runs or states what
//! holds: the condition of a `while`, a `guard` or an `if`; the scrutinee of a `match` on a
//! `bool`, an `if` spelled another way ([`decides`]); a contract clause, `requires`, `ensures`,
//! `invariant` or `decreases`; and each operand of `and` and `or`, one of which runs only by the
//! other's value. None holds an assignment or passes a place `mut`, anywhere inside it. Where one
//! did, every reader of the program, Tier 1 first, had to tell which of its evaluations a later
//! state had run: a right operand that ran or not, a branch taken or not, a round cut short, a
//! clause evaluated or erased. The rule keeps that question from arising.
//!
//! A condition with an effect is `E0236`, reported once, at its first effect, for the outermost
//! condition that holds it. Where every effect lies in one call that the condition evaluates
//! first, the message names that call: it moves into a `val` on a line of its own, and the
//! condition's comparisons stay where they are, so that Tier 1 keeps what they tell it. Where
//! that call is also the first thing its statement evaluates, every time the statement runs,
//! the fix does so. A loop's condition runs every round, so its message shows the loop that
//! keeps that, splitting a chain of `and` where the call stands. A clause has no fix: a contract
//! states what holds without changing it.

use mente_syntax::diagnostics::{Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    Arg, ArmBody, BinaryOp, BlockId, Body, ContractKind, Else, ExprId, ExprKind, FnSyntax, Payload,
    StmtId, StmtKind, Tree,
};
use mente_syntax::Span;

use crate::check::Typing;
use crate::names::Resolution;
use crate::types::Ty;
use crate::wide::line_before;

/// Whether a `match` on `scrutinee` decides as an `if` does, so that its scrutinee is a
/// condition: a scrutinee of type `bool`. Tier 1 reads the same definition.
pub(crate) fn decides(typing: &Typing, scrutinee: ExprId) -> bool {
    matches!(typing.exprs.get(scrutinee.0 as usize), Some(Ty::Bool))
}

/// Every condition of `f` that assigns or passes a place `mut`, each as `E0236`.
pub(crate) fn effects_in_conditions(
    f: &FnSyntax,
    resolution: &Resolution,
    typing: &Typing,
    src: &str,
) -> Vec<Diagnostic> {
    let mut w = Walk {
        tree: &f.tree,
        typing,
        open: None,
        found: Vec::new(),
    };
    for c in &f.contracts {
        let kw = match c.kind {
            ContractKind::Requires => "requires",
            ContractKind::Ensures => "ensures",
            ContractKind::Decreases => "decreases",
        };
        w.within(Place::Clause(kw), c.expr, None);
    }
    match f.body {
        Some(Body::Block(b)) => w.block(b),
        Some(Body::Expr(e)) => w.expr(e),
        None => {}
    }
    let mut given: Vec<String> = Vec::new();
    w.found
        .into_iter()
        .map(|open| report(&f.tree, resolution, src, open, &mut given))
        .collect()
}

/// Which condition a place is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// A loop's condition, evaluated before every round.
    While,
    Guard,
    If,
    /// An `else if`'s, evaluated only where the conditions before it fail.
    ElseIf,
    /// The scrutinee of a `match` on a `bool`.
    Scrutinee,
    /// A contract clause, by its keyword.
    Clause(&'static str),
    /// The operands of an `and` or an `or`.
    Operand(BinaryOp),
}

impl Place {
    /// The words that name the condition in a message.
    fn subject(self) -> String {
        match self {
            Place::While => "the condition of this `while`".to_string(),
            Place::Guard => "the condition of this `guard`".to_string(),
            Place::If => "the condition of this `if`".to_string(),
            Place::ElseIf => "the condition of this `else if`".to_string(),
            Place::Scrutinee => "the scrutinee of this `match` on a `bool`".to_string(),
            Place::Clause(kw) => format!("this `{}` clause", kw),
            Place::Operand(BinaryOp::Or) => "an operand of this `or`".to_string(),
            Place::Operand(_) => "an operand of this `and`".to_string(),
        }
    }

    /// Whether a call the condition evaluates first may move to the line before its statement
    /// and keep when it runs: not a loop's condition, which runs every round, nor an `else if`'s,
    /// which runs only where the conditions before it fail, nor a clause, which a proof erases.
    fn movable(self) -> bool {
        match self {
            Place::Guard | Place::If | Place::Scrutinee | Place::Operand(_) => true,
            Place::While | Place::ElseIf | Place::Clause(_) => false,
        }
    }

    /// Why the place may change nothing.
    fn reason(self) -> &'static str {
        match self {
            Place::Clause(_) => "a contract states what holds without changing it",
            Place::Operand(_) => {
                "an operand of `and` or `or` changes nothing, since one runs only by the other's value"
            }
            Place::While
            | Place::Guard
            | Place::If
            | Place::ElseIf
            | Place::Scrutinee => "a condition only decides",
        }
    }
}

/// Where an expression stands in its statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum At {
    /// The first thing the statement at this span evaluates, every time it runs: an `if` or a
    /// `match` statement, or what a statement binds, discards or returns.
    First(Span),
    /// The `if` of an `else if`.
    ElseIf,
    /// Anywhere else.
    Inner,
}

/// What a condition changes, and the call it passes a place `mut` to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Effect {
    /// The place assigned or passed `mut`.
    span: Span,
    passes_mut: bool,
    call: Option<ExprId>,
}

/// A condition being walked, or walked and found with effects.
struct Open {
    place: Place,
    root: ExprId,
    /// The statement whose first evaluation the condition is, if it is one.
    first: Option<Span>,
    effects: Vec<Effect>,
}

struct Walk<'a> {
    tree: &'a Tree,
    typing: &'a Typing,
    /// The outermost condition being walked.
    open: Option<Open>,
    /// Each condition with an effect.
    found: Vec<Open>,
}

impl Walk<'_> {
    fn block(&mut self, b: BlockId) {
        for &s in &self.tree.block(b).stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: StmtId) {
        let stmt = self.tree.stmt(s);
        let first = At::First(stmt.span);
        match &stmt.kind {
            StmtKind::Binding { init, .. } => self.expr_at(*init, first),
            StmtKind::Assign { place, value, .. } => {
                self.effect(Effect {
                    span: self.tree.expr(*place).span,
                    passes_mut: false,
                    call: None,
                });
                self.expr(*place);
                self.expr(*value);
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                self.within(Place::While, *cond, None);
                for &e in invariants {
                    self.within(Place::Clause("invariant"), e, None);
                }
                for &e in decreases {
                    self.within(Place::Clause("decreases"), e, None);
                }
                self.block(*body);
            }
            StmtKind::Guard { cond, otherwise } => {
                self.within(Place::Guard, *cond, Some(stmt.span));
                self.block(*otherwise);
            }
            StmtKind::Return(value) => {
                if let Some(e) = value {
                    self.expr_at(*e, first);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Discard(e) | StmtKind::Expr(e) => self.expr_at(*e, first),
        }
    }

    /// An expression that stands inside another.
    fn expr(&mut self, e: ExprId) {
        self.expr_at(e, At::Inner);
    }

    fn expr_at(&mut self, e: ExprId, at: At) {
        let first = match at {
            At::First(s) => Some(s),
            At::ElseIf | At::Inner => None,
        };
        match &self.tree.expr(e).kind {
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
            ExprKind::Unary { operand, .. } => self.expr(*operand),
            ExprKind::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                ..
            } => self.within(Place::Operand(*op), e, first),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(*lhs);
                self.expr(*rhs);
            }
            ExprKind::Index { base, index } => {
                self.expr(*base);
                self.expr(*index);
            }
            ExprKind::Field { base, .. } => self.expr(*base),
            ExprKind::Call { args, .. } => self.args(args, Some(e)),
            ExprKind::Construct { fields, .. } => {
                for f in fields {
                    self.expr(f.value);
                }
            }
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => {}
                Payload::Named(fields) => {
                    for f in fields {
                        self.expr(f.value);
                    }
                }
                Payload::Positional(args) => self.args(args, None),
            },
            ExprKind::Match { scrutinee, arms } => {
                if decides(self.typing, *scrutinee) {
                    self.within(Place::Scrutinee, *scrutinee, first);
                } else {
                    self.expr(*scrutinee);
                }
                for arm in arms {
                    match arm.body {
                        ArmBody::Expr(x) => self.expr(x),
                        ArmBody::Block(b) => self.block(b),
                    }
                }
            }
            ExprKind::Array(elements) => {
                for &x in elements {
                    self.expr(x);
                }
            }
            ExprKind::Repeat { value, .. } => self.expr(*value),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let place = match at {
                    At::ElseIf => Place::ElseIf,
                    At::First(_) | At::Inner => Place::If,
                };
                self.within(place, *cond, first);
                self.block(*then);
                match otherwise {
                    None => {}
                    Some(Else::Block(b)) => self.block(*b),
                    Some(Else::If(x)) => self.expr_at(*x, At::ElseIf),
                }
            }
        }
    }

    fn args(&mut self, args: &[Arg], call: Option<ExprId>) {
        for a in args {
            if a.is_mut {
                self.effect(Effect {
                    span: self.tree.expr(a.value).span,
                    passes_mut: true,
                    call,
                });
            }
            self.expr(a.value);
        }
    }

    /// The condition `root` at `place`, whose statement it is the first evaluation of when
    /// `first` says so; inside another condition, a part of that one.
    fn within(&mut self, place: Place, root: ExprId, first: Option<Span>) {
        let opened = self.open.is_none();
        if opened {
            self.open = Some(Open {
                place,
                root,
                first,
                effects: Vec::new(),
            });
        }
        // An `and` or an `or` is its own root: its operands are what it holds.
        match &self.tree.expr(root).kind {
            ExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                lhs,
                rhs,
            } => {
                let (lhs, rhs) = (*lhs, *rhs);
                self.expr(lhs);
                self.expr(rhs);
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
            | ExprKind::Error => self.expr(root),
        }
        if opened {
            if let Some(open) = self.open.take().filter(|o| !o.effects.is_empty()) {
                self.found.push(open);
            }
        }
    }

    fn effect(&mut self, effect: Effect) {
        if let Some(open) = &mut self.open {
            open.effects.push(effect);
        }
    }
}

/// The call that holds every effect of a condition, when one does.
fn holding_call(tree: &Tree, effects: &[Effect]) -> Option<ExprId> {
    let call = effects.first()?.call?;
    let at = tree.expr(call).span;
    effects
        .iter()
        .all(|e| at.start <= e.span.start && e.span.end <= at.end)
        .then_some(call)
}

/// Whether `call` is the first thing `e` evaluates: `e` itself, or the first thing its first
/// operand evaluates.
fn evaluated_first(tree: &Tree, e: ExprId, call: ExprId) -> bool {
    if e == call {
        return true;
    }
    match &tree.expr(e).kind {
        ExprKind::Binary { lhs, .. } => evaluated_first(tree, *lhs, call),
        ExprKind::Unary { operand, .. } => evaluated_first(tree, *operand, call),
        ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => {
            evaluated_first(tree, *base, call)
        }
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => false,
    }
}

/// The conjuncts of `e`, a chain of `and` read left to right, or `e` alone.
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

/// What a loop's condition that changes state is told where no call can be named.
const EVERY_ROUND: &str = "move it to the top of the loop's body, which runs every round, and leave the loop there with `guard ... else { break }`";

/// `E0236` for the condition `open`, under a name for its fix that no binding, no name of the
/// module and no earlier fix of the function in `given` has.
fn report(
    tree: &Tree,
    resolution: &Resolution,
    src: &str,
    open: Open,
    given: &mut Vec<String>,
) -> Diagnostic {
    let text = |s: Span| src.get(s.start..s.end).unwrap_or("").to_string();
    let first = open.effects[0];
    let what = match first.passes_mut {
        true => format!("passes `{}` as `mut`", text(first.span)),
        false => format!("assigns `{}`", text(first.span)),
    };
    let root = tree.expr(open.root).span;
    let name = resolution.unused_name_besides("v", given);
    let call = holding_call(tree, &open.effects);
    // The text of `e` with the call replaced by the name.
    let with_name = |e: ExprId, call: ExprId| {
        let (at, c) = (tree.expr(e).span, tree.expr(call).span);
        format!(
            "{}{}{}",
            src.get(at.start..c.start).unwrap_or(""),
            name,
            src.get(c.end..at.end).unwrap_or("")
        )
    };
    let leading = call.filter(|&c| evaluated_first(tree, open.root, c));
    let how = match (open.place, call) {
        (Place::Clause(_), _) => "write it over the values as they are".to_string(),
        (Place::While, None) => EVERY_ROUND.to_string(),
        (Place::While, Some(c)) => {
            let mut parts = Vec::new();
            conjuncts(tree, open.root, &mut parts);
            let at = tree.expr(c).span;
            let k = parts.iter().position(|&p| {
                let s = tree.expr(p).span;
                s.start <= at.start && at.end <= s.end
            });
            match k.filter(|&k| evaluated_first(tree, parts[k], c)) {
                Some(k) => {
                    let head: Vec<String> =
                        parts[..k].iter().map(|&p| text(tree.expr(p).span)).collect();
                    let mut test = vec![with_name(parts[k], c)];
                    test.extend(parts[k + 1..].iter().map(|&p| text(tree.expr(p).span)));
                    format!(
                        "to call `{}` every round, write `while {} {{` and start its body with `val {} = {}` and then `guard {} else {{ break }}`",
                        text(at),
                        match head.is_empty() {
                            true => "true".to_string(),
                            false => head.join(" and "),
                        },
                        name,
                        text(at),
                        test.join(" and ")
                    )
                }
                None => EVERY_ROUND.to_string(),
            }
        }
        (
            Place::Guard | Place::If | Place::ElseIf | Place::Scrutinee | Place::Operand(_),
            _,
        ) => match leading {
            Some(c) => format!(
                "compute `{}` into a `val` in a statement of its own where it runs, `val {} = {}`, and test `{}`",
                text(tree.expr(c).span),
                name,
                text(tree.expr(c).span),
                with_name(open.root, c)
            ),
            None => "move it out of the condition into a statement of its own where it runs, and test what it leaves".to_string(),
        },
    };
    let mut d = Diagnostic::error(
        "E0236",
        first.span,
        format!(
            "{} {}, and {}: {}",
            open.place.subject(),
            what,
            open.place.reason(),
            how
        ),
    )
    .with_related(root, "the condition".to_string());
    // The fix moves the call to the line before its statement, where it runs as it did: first,
    // every time the statement runs.
    let fix = open
        .first
        .zip(leading)
        .filter(|_| open.place.movable())
        .and_then(|(stmt, c)| {
            let line = format!("val {} = {}", name, text(tree.expr(c).span));
            line_before(src, stmt.start, &line).map(|insert| (insert, c))
        });
    if let Some((insert, c)) = fix {
        d.fixes.push(Fix {
            message: format!(
                "compute `{}` into `{}` on the line before",
                text(tree.expr(c).span),
                name
            ),
            applicability: Applicability::MachineApplicable,
            edits: vec![
                insert,
                Edit {
                    span: tree.expr(c).span,
                    replacement: name.clone(),
                },
            ],
        });
        given.push(name);
    }
    d
}

#[cfg(test)]
mod tests {
    use mente_syntax::diagnostics::{apply_fix, Applicability, Diagnostic};

    use crate::check_source;

    const NEXT: &str = "fn next(mut i: u8) -> u8 {\n  i = i / 2\n  i\n}\nfn bump(mut x: u8) -> u8 {\n  x = 9\n  0\n}\nfn zap(mut z: u8) -> u8 {\n  z = 7\n  0\n}\n";

    fn module(body: &str) -> String {
        format!("module t\n{}{}\n", NEXT, body)
    }

    fn diagnostics(body: &str) -> Vec<Diagnostic> {
        check_source(&module(body)).err().unwrap_or_default()
    }

    /// The one diagnostic of `body`, which must be `E0236`, with the text at its span.
    fn refused(body: &str) -> (Diagnostic, String) {
        let src = module(body);
        let d = check_source(&src).err().unwrap_or_default();
        assert_eq!(
            d.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["E0236"],
            "{}\n{:?}",
            body,
            d
        );
        let at = src[d[0].primary.start..d[0].primary.end].to_string();
        (d[0].clone(), at)
    }

    #[test]
    fn test_a_condition_that_changes_state_is_e0236() {
        // A loop's condition, which runs every round: the message splits the chain of `and` where
        // the call stands, and offers no fix.
        let (d, at) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  var r: u8 = 0\n  while r < 8 and next(mut i) > 0 {\n    r += 1\n  }\n  r\n}",
        );
        assert_eq!(at, "i");
        assert_eq!(
            d.message,
            "the condition of this `while` passes `i` as `mut`, and a condition only decides: to call `next(mut i)` every round, write `while r < 8 {` and start its body with `val v = next(mut i)` and then `guard v > 0 else { break }`"
        );
        assert!(d.fixes.is_empty());
        assert_eq!(d.related.len(), 1);
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  var r: u8 = 0\n  while next(mut i) > 0 and r < 8 {\n    r += 1\n  }\n  r\n}",
        );
        assert!(
            d.message.ends_with("write `while true {` and start its body with `val v = next(mut i)` and then `guard v > 0 and r < 8 else { break }`"),
            "{}",
            d.message
        );
        // An assignment in a block of an `if` inside it: the measure-reset reproduction of the
        // review of pull request 193.
        let (d, at) = refused(
            "pub fn f(q: u8) -> u8 {\n  var c: u8 = 0\n  while (if q > 8 {\n    c = 0\n    0\n  } else {\n    0\n  }) == 0 and c < 3 decreases 3 - c {\n    c += 1\n  }\n  c\n}",
        );
        assert_eq!(at, "c");
        assert_eq!(
            d.message,
            "the condition of this `while` assigns `c`, and a condition only decides: move it to the top of the loop's body, which runs every round, and leave the loop there with `guard ... else { break }`"
        );
        // A call that is not what the condition evaluates first, here inside a `match`, is not
        // named: moving it would change what runs before it.
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  guard (match next(mut i) {\n    0 => false\n    _ => true\n  }) else {\n    return 0\n  }\n  i\n}",
        );
        assert_eq!(
            d.message,
            "the condition of this `guard` passes `i` as `mut`, and a condition only decides: move it out of the condition into a statement of its own where it runs, and test what it leaves"
        );
        assert!(d.fixes.is_empty());
        // An `if` statement whose call runs first is offered the fix.
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  if next(mut i) > 3 {\n    return 1\n  }\n  i\n}",
        );
        assert_eq!(
            d.message,
            "the condition of this `if` passes `i` as `mut`, and a condition only decides: compute `next(mut i)` into a `val` in a statement of its own where it runs, `val v = next(mut i)`, and test `v > 3`"
        );
        assert_eq!(d.fixes.len(), 1);
        // An `else if` runs only where the conditions before it fail, and an `if` inside an
        // expression after what it follows: neither is offered a fix.
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  if n > 9 {\n    return 1\n  } else if next(mut i) > 3 {\n    return 2\n  }\n  i\n}",
        );
        assert!(d
            .message
            .starts_with("the condition of this `else if` passes `i` as `mut`"));
        assert!(d.fixes.is_empty());
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  val k: u8 = i + (if next(mut i) > 3 { 1 } else { 0 })\n  k\n}",
        );
        assert!(d
            .message
            .starts_with("the condition of this `if` passes `i` as `mut`"));
        assert!(d.fixes.is_empty());
        // A `match` on a `bool` is an `if` spelled another way: its scrutinee is a condition, the
        // fix offered where its call runs first.
        let (d, at) = refused(
            "pub fn f() -> u8 {\n  var x: u8 = 0\n  val k = match x == 0 and bump(mut x) == 0 {\n    true => x\n    false => 0\n  }\n  k\n}",
        );
        assert_eq!(at, "x");
        assert!(d
            .message
            .starts_with("the scrutinee of this `match` on a `bool` passes `x` as `mut`, and a condition only decides: move it out"));
        assert!(d.fixes.is_empty());
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  val k: u8 = match next(mut i) > 3 {\n    true => 1\n    false => 0\n  }\n  k + i\n}",
        );
        assert_eq!(d.fixes.len(), 1);
        // Each contract clause, of a function or of a loop, with no fix.
        for (body, kw) in [
            ("pub fn f(mut n: u8) -> u8\n  requires next(mut n) > 0\n= n", "requires"),
            ("pub fn f(mut n: u8) -> u8\n  ensures bump(mut n) == 0\n= n", "ensures"),
            (
                "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  var j = n\n  while i < 3 invariant next(mut j) >= 0 {\n    i += 1\n  }\n  i\n}",
                "invariant",
            ),
            (
                "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  var j = n\n  while i < 3 decreases 3 - i + next(mut j) * 0 {\n    i += 1\n  }\n  i\n}",
                "decreases",
            ),
        ] {
            let (d, _) = refused(body);
            assert!(
                d.message.starts_with(&format!("this `{}` clause passes", kw))
                    && d.message.ends_with(
                        ", and a contract states what holds without changing it: write it over the values as they are"
                    ),
                "{}",
                d.message
            );
            assert!(d.fixes.is_empty());
        }
        // An operand of `and` or `or`, in a value, offered the fix where its call runs first.
        let (d, _) = refused(
            "pub fn f(n: u8) -> bool {\n  var i = n\n  val c = n > 3 and next(mut i) > 0\n  c\n}",
        );
        assert!(d.message.starts_with(
            "an operand of this `and` passes `i` as `mut`, and an operand of `and` or `or` changes nothing, since one runs only by the other's value: move it out"
        ));
        assert!(d.fixes.is_empty());
        let (d, _) = refused(
            "pub fn f(n: u8) -> bool {\n  var i = n\n  val c = next(mut i) > 0 or n > 3\n  c\n}",
        );
        assert!(d
            .message
            .starts_with("an operand of this `or` passes `i` as `mut`"));
        assert_eq!(d.fixes.len(), 1);
        // One diagnostic for a condition, at its first effect, for the outermost condition that
        // holds it: here the `or`, whose left operand is an `and` whose right one calls `bump`.
        let (d, at) = refused(
            "pub fn f() -> u8 {\n  var x: u8 = 0\n  var z: u8 = 0\n  _ = (x == 0 and bump(mut x) == 0) or zap(mut z) == 0\n  z\n}",
        );
        assert_eq!(at, "x");
        assert!(d
            .message
            .starts_with("an operand of this `or` passes `x` as `mut`"));
        let (d, _) = refused(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  while (if next(mut i) > 0 {\n    i = 0\n    true\n  } else {\n    false\n  }) {\n    i = 1\n  }\n  i\n}",
        );
        assert!(d
            .message
            .starts_with("the condition of this `while` passes `i` as `mut`"));
        // An assignment counts even to a binding the condition declares itself.
        let (d, at) = refused(
            "pub fn f(c: bool) -> u8 {\n  var k: u8 = 0\n  if (if c {\n    var t: u8 = 4\n    t = 1\n    t > 0\n  } else {\n    false\n  }) {\n    k = 1\n  }\n  k\n}",
        );
        assert_eq!(at, "t");
        assert!(d
            .message
            .starts_with("the condition of this `if` assigns `t`"));
    }

    #[test]
    fn test_the_fix_moves_the_call_to_the_line_before() {
        let fixed = |body: &str| {
            let src = module(body);
            let d = check_source(&src).err().unwrap_or_default();
            assert_eq!(d.len(), 1, "{:?}", d);
            assert_eq!(
                d[0].fixes[0].applicability,
                Applicability::MachineApplicable
            );
            let fixed = apply_fix(&src, &d[0].fixes[0]).unwrap_or_default();
            assert!(check_source(&fixed).is_ok(), "{}", fixed);
            (d[0].fixes[0].message.clone(), fixed)
        };
        // The comparisons stay in the condition, so that Tier 1 keeps what they say: here that
        // `n` lies below 4 in the branch, which a `val` of the whole condition would lose.
        let (message, text) = fixed(
            "pub fn f(n: u8) -> u8[0..4] {\n  var i = n\n  if next(mut i) < 4 and n < 4 {\n    return n\n  }\n  0\n}",
        );
        assert_eq!(message, "compute `next(mut i)` into `v` on the line before");
        assert!(
            text.contains("  var i = n\n  val v = next(mut i)\n  if v < 4 and n < 4 {\n"),
            "{}",
            text
        );
        // A `guard` that shares its line, under a name no binding of the function has.
        let (_, text) = fixed(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  val v: u8 = 1\n  if n > 0 { guard next(mut i) > 3 else {\n    return v\n  } }\n  i\n}",
        );
        assert!(
            text.contains("  if n > 0 { val v2 = next(mut i)\n  guard v2 > 3 else {\n"),
            "{}",
            text
        );
        // Nor a name of a function, a struct or an enum of the module (`E0220`).
        let (_, text) = fixed(
            "pub fn v() -> u8 = 1\npub struct v2 {\n  a: u8\n}\npub enum v3 {\n  A\n  B\n}\npub fn f(n: u8) -> u8 {\n  var i = n\n  if next(mut i) > 3 {\n    return v()\n  }\n  i\n}",
        );
        assert!(text.contains("  val v4 = next(mut i)\n"), "{}", text);
        // A `bool` scrutinee and an operand of `or` that a binding's statement evaluates first.
        let (_, text) = fixed(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  val k: u8 = match next(mut i) > 3 {\n    true => 1\n    false => 0\n  }\n  k + i\n}",
        );
        assert!(
            text.contains("  val v = next(mut i)\n  val k: u8 = match v > 3 {\n"),
            "{}",
            text
        );
        let (_, text) = fixed(
            "pub fn f(n: u8) -> bool {\n  var i = n\n  val c = next(mut i) > 0 or n > 3\n  c\n}",
        );
        assert!(
            text.contains("  val v = next(mut i)\n  val c = v > 0 or n > 3\n"),
            "{}",
            text
        );
        // Two fixes of one function declare two names, so that applying both declares none twice.
        let d = diagnostics(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  if next(mut i) > 3 {\n    return 1\n  }\n  if next(mut i) > 2 {\n    return 2\n  }\n  i\n}",
        );
        let names: Vec<&str> = d.iter().map(|d| d.fixes[0].message.as_str()).collect();
        assert_eq!(
            names,
            [
                "compute `next(mut i)` into `v` on the line before",
                "compute `next(mut i)` into `v2` on the line before"
            ]
        );
    }

    #[test]
    fn test_effects_outside_conditions_stay() {
        // The loop that calls every round, an assignment in an `if`'s branch, a comparison of a
        // call bound to a name, and a `mut` call in a scrutinee and an arm of a `match` on an
        // integer, which is evaluated once and narrowed by its patterns.
        let d = diagnostics(
            "pub fn f(n: u8) -> u8 {\n  var i = n\n  var r: u8 = 0\n  while r < 8 {\n    val v = next(mut i)\n    guard v > 0 else {\n      break\n    }\n    r += 1\n  }\n  if r > 2 {\n    i = 0\n  }\n  val big = next(mut i) > 3\n  val k = match next(mut i) {\n    0 => next(mut i)\n    _ => i\n  }\n  if big {\n    return k\n  }\n  k\n}",
        );
        assert!(d.is_empty(), "{:?}", d);
    }
}

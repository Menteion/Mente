//! What Tier 1 claims at each point it analyses, for the containment oracle, which checks the
//! claims against the reference interpreter's runs of the generated programs and the conformance
//! suite (`mente-cli`, `mir_run::containment`;
//! [`tier1-containment`](../../../docs/design-review.md#tier1-containment)).
//!
//! [`super::check_with`] writes them when [`super::Ask::states`] asks, in its recording pass
//! alone, the one whose obligations decide which checks the lowering leaves out: Houdini's rounds
//! at a loop head and every evaluation [`super::Fx::quietly`] makes write none. Each point holds
//! its state cut down to the variables of the bindings in scope there, each an integer binding or
//! a scalar field path of a struct binding, which the interpreter's values of the same bindings
//! are compared with.
//!
//! Cutting a state down is sound: every constraint written here is one the state holds, an entry
//! of its matrix or a bound kept beside it, so a value that breaks one lies outside the state, and
//! the oracle reports no value the state admits. It is exact for the matrix: Tier 1 keeps the
//! octagon tightly closed, so every bound the matrix implies between two variables, or on one, is
//! one of its entries, and the entries over the kept variables are the matrix with every other
//! variable left free (Mine, "The octagon abstract domain", 2006, projection by strong closure).
//! It is not exact beyond the matrix: a bound kept beside it, past its reach, is not carried along
//! a relation, and past `RELATIONAL_LIMIT` variables the octagon is a box, which keeps no relation.
//! What the oracle misses there it misses for want of a constraint, never by a false one.

use mente_syntax::tree::{BlockId, ExprId, StmtId};

use crate::octagon::{Exact, Octagon, Term, INF};

/// A point of a function Tier 1 records its state at, keyed as the reference interpreter shows
/// it (`mente_interp::Event`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Point {
    /// Before a statement runs.
    Before(StmtId),
    /// At the end of a block that ran to its end.
    End(BlockId),
    /// At the head of a round of a `while` loop, before its condition.
    Head(StmtId),
}

/// A variable Tier 1 follows in its octagon: an integer binding's, or a scalar field path's of
/// a binding of a struct type, `p.a.b` as `["a", "b"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracked {
    pub var: usize,
    pub binding: usize,
    pub path: Vec<String>,
}

/// A term of a relation: a variable, or its negation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signed {
    pub var: usize,
    pub neg: bool,
}

/// A state cut down to the tracked variables of the bindings in scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Whether the state is empty: no path reaches the point.
    pub empty: bool,
    /// Each tracked variable in scope with its least and greatest value, `None` for none.
    pub bounds: Vec<(usize, Option<Exact>, Option<Exact>)>,
    /// Each relation `a + b <= c` between two of them.
    pub relations: Vec<(Signed, Signed, i128)>,
}

/// The value of an integer expression where Tier 1 evaluated it: whether the state there is
/// empty, and its least and greatest value, `None` for none; an array's is its elements'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value {
    pub expr: ExprId,
    pub empty: bool,
    pub lo: Option<Exact>,
    pub hi: Option<Exact>,
}

/// Everything one function's recording pass claimed: its tracked variables, the state at each
/// point, as many times as the pass reached the point, and each integer expression's value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct States {
    pub tracked: Vec<Tracked>,
    /// The roots of the parts of the tree the walk never enters, by design, though a run
    /// evaluates them: each `ensures` outside the Tier-1 fragment, which `@checked` checks at run
    /// time and `@prove` trusts. A point inside one has no state, and that is no claim.
    pub unwalked: Vec<ExprId>,
    pub points: Vec<(Point, Facts)>,
    pub values: Vec<Value>,
}

/// `v` as a bound, `None` at or beyond the domain's limit, where a bound is no bound.
pub(super) fn bound(v: i128) -> Option<Exact> {
    (-INF < v && v < INF).then(|| Exact::from_i128(v))
}

/// `st` cut down to `vars`: their bounds, beyond the matrix's reach included, and the bound of
/// `±a ±b` for each two of them that the matrix holds, read entry by entry, so that a point costs
/// the square of the variables in scope, not of the function's.
pub(super) fn facts(st: &Octagon, vars: &[usize]) -> Facts {
    let bounds: Vec<(usize, Option<Exact>, Option<Exact>)> = vars
        .iter()
        .map(|&v| {
            let (lo, hi) = st.exact_bounds(v);
            (v, lo, hi)
        })
        .collect();
    // Bounds that cross without the flag, after a slip of the closure, refuse every value, so
    // the oracle reports any value read there all the same.
    if st.is_empty() {
        return Facts {
            empty: true,
            ..Facts::default()
        };
    }
    let mut relations = Vec::new();
    // A variable that holds an evaluation is a site's, never a binding's; none is read here. A
    // box's bound of two terms is the sum of each one's, implied as every other is.
    let plain: Vec<usize> = vars
        .iter()
        .copied()
        .filter(|&v| st.stamp_of(v) == 0)
        .collect();
    let term = |var: usize, neg: bool| {
        let t = Term::pos(var);
        if neg {
            t.negated()
        } else {
            t
        }
    };
    for (i, &a) in plain.iter().enumerate() {
        for &b in &plain[i + 1..] {
            for (na, nb) in [(false, false), (false, true), (true, false), (true, true)] {
                let c = st.upper(&[term(a, na), term(b, nb)]);
                if c < INF {
                    relations.push((Signed { var: a, neg: na }, Signed { var: b, neg: nb }, c));
                }
            }
        }
    }
    Facts {
        empty: false,
        bounds,
        relations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_syntax::tree::Tree;

    const SRC: &str = "module m\nstruct P {\n  a: u8\n}\npub fn f(x: u8) -> u8 {\n  val y: u16 = x + 1\n  var i: u8 = 0\n  val p = P(a: x)\n  val k: [2]u8 = [x, 3]\n  while i < x {\n    i = i + 1\n  }\n  return k[1] + (y - 1 - x) + p.a\n}\npub fn g(x: u8) -> u8 {\n  if x > 7 { todo(\"big\") } else { x }\n}\n@checked\npub fn h(x: u8) -> u8\n  ensures result <= 127\n  ensures (if result > 3 { result > 3 } else { result <= 3 })\n{\n  val y: u8 = x / 2\n  y\n}\npub fn n(x: u8) -> u8 {\n  guard x > 0 else { return 1 }\n  match x {\n    0 => 7\n    _ => x\n  }\n}\npub fn w(a: u128, b: u8) -> u8 {\n  val c: u8 = b\n  c\n}\n";

    fn exact(v: i128) -> Option<Exact> {
        Some(Exact::from_i128(v))
    }

    fn states() -> Vec<(String, States, Tree, BlockId)> {
        let module = match crate::check_program(SRC) {
            Ok(m) => m,
            Err(d) => panic!("{:?}", d),
        };
        crate::tier1_states(SRC)
            .into_iter()
            .map(|(name, st)| {
                let i = module.index(&name).unwrap();
                let syntax = &module.functions[i].as_ref().unwrap().syntax;
                let Some(mente_syntax::tree::Body::Block(body)) = syntax.body else {
                    panic!("{} has no block body", name);
                };
                (name, st, syntax.tree.clone(), body)
            })
            .collect()
    }

    fn expr(tree: &Tree, text: &str) -> ExprId {
        (0..tree.exprs.len() as u32)
            .map(ExprId)
            .find(|&e| {
                let sp = tree.expr(e).span;
                &SRC[sp.start..sp.end] == text
            })
            .unwrap()
    }

    fn at(st: &States, p: Point) -> Vec<&Facts> {
        st.points
            .iter()
            .filter(|(q, _)| *q == p)
            .map(|(_, f)| f)
            .collect()
    }

    #[test]
    fn test_the_recording_pass_writes_its_state_at_each_point_it_reaches() {
        let all = states();
        let (_, f, tree, body_id) = &all[0];
        // x, y and i by their variables, and p's field a by its path; the array k has none.
        let tracked: Vec<(usize, Vec<String>)> = f
            .tracked
            .iter()
            .map(|t| (t.binding, t.path.clone()))
            .collect();
        assert_eq!(
            tracked,
            vec![
                (0, vec![]),
                (1, vec![]),
                (2, vec![]),
                (3, vec!["a".to_string()])
            ]
        );
        let var = |b: usize| f.tracked.iter().find(|t| t.binding == b).unwrap().var;
        let (x, y, i) = (var(0), var(1), var(2));
        let body = tree.block(*body_id);
        let stmt = |k: usize| body.stmts[k];
        // Before `var i`: x and y in scope, y exactly x + 1, and every relation between them.
        let before_i = at(f, Point::Before(stmt(1)));
        assert_eq!(before_i.len(), 1);
        let s = |var, neg| Signed { var, neg };
        assert_eq!(
            *before_i[0],
            Facts {
                empty: false,
                bounds: vec![(x, exact(0), exact(255)), (y, exact(1), exact(256))],
                relations: vec![
                    (s(x, false), s(y, false), 511),
                    (s(x, false), s(y, true), -1),
                    (s(x, true), s(y, false), 1),
                    (s(x, true), s(y, true), -1),
                ],
            }
        );
        // Before the loop, i is 0 exactly, which no bound of a crossed state could hold, and
        // p's field is in scope.
        let before_loop = at(f, Point::Before(stmt(4)));
        assert_eq!(before_loop.len(), 1);
        assert!(!before_loop[0].empty);
        assert!(before_loop[0].bounds.contains(&(i, exact(0), exact(0))));
        assert!(before_loop[0].bounds.iter().any(|b| b.0 == var(3)));
        // The head is written once, by the recording pass alone, and i lies in its type there.
        let head = at(f, Point::Head(stmt(4)));
        assert_eq!(head.len(), 1, "{:?}", head);
        assert!(head[0].bounds.contains(&(i, exact(0), exact(255))));
        // After `return`, the body's end is empty.
        let end = at(f, Point::End(*body_id));
        assert!(end.len() == 1 && end[0].empty, "{:?}", end);
        // Values: an operation's bounds.
        let value = |text: &str| {
            let e = expr(tree, text);
            f.values
                .iter()
                .filter(|v| v.expr == e)
                .copied()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            value("x + 1"),
            vec![Value {
                expr: expr(tree, "x + 1"),
                empty: false,
                lo: exact(1),
                hi: exact(256)
            }]
        );
        // An array literal entering an array type is evaluated element by element, and has no
        // value of its own; a read of the array has its elements'.
        assert!(value("[x, 3]").is_empty());
        let array = value("k");
        assert!(
            array.len() == 1 && array[0].lo == exact(0) && array[0].hi == exact(255),
            "{:?}",
            array
        );
        // A value in a loop's body is written once, by the recording pass, from the head it
        // settles on, though Houdini's rounds evaluate it too.
        let step = value("i + 1");
        assert!(
            step.len() == 1 && step[0].lo == exact(1) && step[0].hi == exact(255),
            "{:?}",
            step
        );
        // `todo` leaves its state empty, and its value is written as one of an empty state.
        let (_, g, gtree, _) = &all[1];
        let todo = expr(gtree, "todo(\"big\")");
        let written: Vec<&Value> = g.values.iter().filter(|v| v.expr == todo).collect();
        assert!(
            written.len() == 1 && written[0].empty && written[0].lo.is_none(),
            "{:?}",
            written
        );
        // A routed `ensures` is walked nowhere, and named as such; one in the fragment is
        // walked, and a function without a routed clause names none.
        assert!(f.unwalked.is_empty() && g.unwalked.is_empty());
        let (_, h, htree, _) = &all[2];
        let routed = expr(htree, "if result > 3 { result > 3 } else { result <= 3 }");
        assert_eq!(h.unwalked, vec![routed]);
        assert!(h.values.iter().all(|v| v.expr != routed));
        assert!(h.values.iter().any(|v| v.expr == expr(htree, "127")));
        // An arm Tier 1 holds unreachable has its value written as one of an empty state.
        let (_, n, ntree, _) = &all[3];
        let arm = expr(ntree, "7");
        let written: Vec<&Value> = n.values.iter().filter(|v| v.expr == arm).collect();
        assert!(written.len() == 1 && written[0].empty, "{:?}", written);
        // A relation the matrix leaves unbounded is no relation: of a `u128` beside a `u8`, only
        // the two with the `u128` negated are written.
        let (_, w, wtree, wbody) = &all[4];
        let (a, b) = (w.tracked[0].var, w.tracked[1].var);
        let before = at(w, Point::Before(wtree.block(*wbody).stmts[0]));
        let s = |var, neg| Signed { var, neg };
        assert_eq!(
            before[0].relations,
            vec![(s(a, true), s(b, false), 255), (s(a, true), s(b, true), 0)]
        );
    }

    #[test]
    fn test_a_bound_at_the_domains_limit_is_no_bound() {
        assert_eq!(bound(INF), None);
        assert_eq!(bound(-INF), None);
        assert_eq!(bound(INF - 1), exact(INF - 1));
        assert_eq!(bound(1 - INF), exact(1 - INF));
        assert_eq!(bound(0), exact(0));
    }
}

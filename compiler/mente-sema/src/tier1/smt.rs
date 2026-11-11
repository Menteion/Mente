//! The SMT cross-check of Tier 1: every obligation Tier 1 discharges, cross-checked in CI by an
//! SMT solver installed on the runner, never in the build (`docs/design-brief.md`, "The quality
//! system").
//!
//! Asked through [`super::check_with_queries`], which `mn smt` calls, Tier 1 writes each
//! obligation it discharges as an SMT-LIB 2 query in QF_LIA, built from the very states it
//! decided the obligation on, so that the query and the decision cannot drift apart:
//!
//! - each binding in scope is an `Int` bounded by its type's range;
//! - each state's constraints follow, as its matrix holds them ([`Octagon::constraints`]):
//!   intervals and octagon constraints `±x ±y <= c` over the bindings and the other values
//!   Tier 1 follows, such as call arguments and results and the values of `if`s, elements and
//!   remainders;
//! - then the obligation, denied: the value outside its refinement or its type, a clause false,
//!   an index outside its array, a divisor zero, a shift amount outside its operand's width, a
//!   measure negative or no smaller. A value Tier 1 knows only by its bounds is a fresh `Int`
//!   between them, a quotient by a constant is a fresh `Int` with the facts of division rounded
//!   toward zero, and a value or a condition Tier 1 does not follow is fresh and unconstrained.
//!
//! An obligation decided on several states, measures that get smaller on every path of a loop's
//! body or a written invariant on entry and after a round, is denied on each of them, as a
//! disjunction.
//!
//! The solver must answer `unsat`. That proves that each obligation follows from the facts Tier 1
//! held where it decided it, so the decision itself is checked by a solver that shares no code
//! with Tier 1: the meaning of each kind of obligation, the closure that adds the denied
//! obligation to the state, and the bounds of quotients and intervals. It does not prove that
//! those facts were derived soundly from the program: the transfer functions, the joins, the
//! closure that built each state and the loop-head facts Houdini keeps are taken as given here.
//! Soundness by execution, which compares Tier 1's verdicts with runs of generated programs,
//! checks how the facts were derived.
//!
//! Where Tier 1's state is empty with no contradiction among its constraints, after a `return`,
//! a `break`, a `continue` or a `todo`, or under a condition on constants that is false, no path
//! reaches the obligation. A query of the state cannot check that, so there is none
//! ([`Query::text`]), and `mn smt` lists the obligation instead.

use std::collections::BTreeSet;
use std::fmt::Write;

use mente_syntax::tree::{BinaryOp, ContractKind, ExprId, StmtKind};
use mente_syntax::Span;

use super::{Cond, Fx, Obligation, Site, Tag, Val};
use crate::names::BindingKind;
use crate::octagon::{Exact, Octagon, Term, INF};
use crate::types::{Int, IntTy, Ty};

/// What an obligation states, for its query to deny.
#[derive(Clone, Debug)]
pub(super) enum Goal {
    /// A value inside an integer type. With no type known, Tier 1 shows it only where no state
    /// reaches it, and the query denies nothing.
    Fits(Val, Option<IntTy>),
    /// An index inside an array of this length.
    Index(Val, u128),
    /// A divisor that is not zero.
    NonZero(Val),
    /// A shift amount below the width, in bits, of its operand's type; 0 when it is not known.
    Shift(Val, u32),
    /// A condition: a value inside its refinement, a contract clause, an invariant, a measure
    /// never negative or smaller.
    Holds(Cond),
}

/// A state Tier 1 decided an obligation on, with the obligation as it stands there.
pub(super) type Path = (Octagon, Goal);

/// An obligation Tier 1 discharged, as the SMT cross-check gets it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// Its index among the obligations of the function's report.
    pub obligation: usize,
    /// The SMT-LIB 2 query, which a solver must answer `unsat`; `None` where no path reaches
    /// the obligation: each state Tier 1 decided it on is empty with no contradiction among its
    /// constraints, which a query of the state cannot check.
    pub text: Option<String>,
}

/// The query of an obligation Tier 1 discharged on `paths`, or `None` where no path reaches it.
pub(super) fn query(fx: &Fx, o: &Obligation, paths: Vec<Path>) -> Option<String> {
    let mut w = Writer::new(fx);
    let total = paths.len();
    let mut cases = Vec::new();
    for (st, goal) in &paths {
        let constraints = st.constraints();
        // A constant constraint is listed only for a diagonal entry below zero: a
        // contradiction among the state's constraints.
        let contradiction = constraints.iter().any(|(t, _)| t.is_empty());
        if st.is_empty() && !contradiction {
            continue;
        }
        let mut facts: Vec<String> = constraints.iter().map(|(t, c)| w.le(t, *c)).collect();
        // The bounds the state keeps beyond the octagon's reach, exactly.
        for (v, lower, upper) in st.far_constraints() {
            let x = w.var(&Term::pos(v));
            if let Some(l) = lower {
                facts.push(format!("(<= {} {})", exact_num(l), x));
            }
            if let Some(u) = upper {
                facts.push(format!("(<= {} {})", x, exact_num(u)));
            }
        }
        let mut denial = Vec::new();
        let denied = w.deny(goal, &mut denial);
        denial.push(denied);
        cases.push((facts, denial));
    }
    if cases.is_empty() {
        return None;
    }
    Some(w.text(o, total, cases))
}

/// An SMT-LIB numeral, negative ones as `(- n)`.
fn num(v: i128) -> String {
    if v < 0 {
        format!("(- {})", v.unsigned_abs())
    } else {
        v.to_string()
    }
}

/// An exact integer as an SMT-LIB numeral, negative ones as `(- n)`.
fn exact_num(v: Exact) -> String {
    let text = v.to_string();
    match text.strip_prefix('-') {
        Some(m) => format!("(- {})", m),
        None => text,
    }
}

/// The first line of a text, for a comment.
fn one_line(text: &str) -> String {
    match text.split_once('\n') {
        Some((first, _)) => format!("{} ...", first.trim_end()),
        None => text.to_string(),
    }
}

/// The SMT-LIB relation of a comparison; `None` for an operator that compares nothing.
fn relation(op: BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Lt => Some("<"),
        BinaryOp::Le => Some("<="),
        BinaryOp::Gt => Some(">"),
        BinaryOp::Ge => Some(">="),
        BinaryOp::Eq => Some("="),
        BinaryOp::Ne => Some("distinct"),
        _ => None,
    }
}

/// The range of an integer type, as SMT-LIB numerals.
fn range(t: IntTy) -> (String, String) {
    let lo = match t.min_magnitude() {
        0 => "0".to_string(),
        m => format!("(- {})", m),
    };
    (lo, t.max().to_string())
}

struct Writer<'f, 'a> {
    fx: &'f Fx<'a>,
    /// The symbol of each octagon variable, and what it stands for.
    names: Vec<(String, String)>,
    /// The octagon variables the query mentions.
    used: BTreeSet<usize>,
    /// The fresh constants: symbol, sort and what each stands for.
    fresh: Vec<(String, &'static str, String)>,
}

impl<'f, 'a> Writer<'f, 'a> {
    fn new(fx: &'f Fx<'a>) -> Writer<'f, 'a> {
        Writer {
            fx,
            names: names(fx),
            used: BTreeSet::new(),
            fresh: Vec::new(),
        }
    }

    fn var(&mut self, t: &Term) -> String {
        self.used.insert(t.var);
        let name = self.names[t.var].0.clone();
        if t.neg {
            format!("(- {})", name)
        } else {
            name
        }
    }

    /// `terms + c`.
    fn sum(&mut self, terms: &[Term], c: i128) -> String {
        let mut parts: Vec<String> = terms.iter().map(|t| self.var(t)).collect();
        if c != 0 || parts.is_empty() {
            parts.push(num(c));
        }
        match parts.as_slice() {
            [one] => one.clone(),
            _ => format!("(+ {})", parts.join(" ")),
        }
    }

    /// `terms <= c`.
    fn le(&mut self, terms: &[Term], c: i128) -> String {
        format!("(<= {} {})", self.sum(terms, 0), num(c))
    }

    fn fresh(&mut self, sort: &'static str, what: String) -> String {
        let name = format!("|?{}|", self.fresh.len());
        self.fresh.push((name.clone(), sort, what));
        name
    }

    /// A value as a term; what a fresh constant must satisfy goes into `side`.
    fn term(&mut self, v: &Val, side: &mut Vec<String>) -> String {
        match v {
            Val::Lin(t, c) => self.sum(t, *c),
            Val::Quot { base, num: n, div } => {
                let q = self.fresh("Int", format!("a quotient by {}, rounded toward zero", div));
                let n = self.sum(&n.0, n.1);
                let dq = format!("(* {} {})", div, q);
                side.push(format!(
                    "(=> (>= {n} 0) (and (<= {dq} {n}) (< {n} (+ {dq} {d}))))",
                    n = n,
                    dq = dq,
                    d = div
                ));
                side.push(format!(
                    "(=> (< {n} 0) (and (< (- {dq} {d}) {n}) (<= {n} {dq})))",
                    n = n,
                    dq = dq,
                    d = div
                ));
                if base.0.is_empty() && base.1 == 0 {
                    q
                } else {
                    format!("(+ {} {})", self.sum(&base.0, base.1), q)
                }
            }
            Val::Itv(lo, hi) => {
                let show = |b: i128| {
                    if b.abs() >= INF {
                        "unbounded".to_string()
                    } else {
                        b.to_string()
                    }
                };
                let x = self.fresh(
                    "Int",
                    format!("a value Tier 1 knows from {} to {}", show(*lo), show(*hi)),
                );
                if *lo > -INF {
                    side.push(format!("(<= {} {})", num(*lo), x));
                }
                if *hi < INF {
                    side.push(format!("(<= {} {})", x, num(*hi)));
                }
                x
            }
            Val::Arr(_) | Val::Other => {
                self.fresh("Int", "a value Tier 1 does not follow".to_string())
            }
        }
    }

    fn cond(&mut self, c: &Cond, side: &mut Vec<String>) -> String {
        match c {
            Cond::Const(b) => b.to_string(),
            Cond::Cmp(op, l, r) => {
                let Some(symbol) = relation(*op) else {
                    return self.fresh("Bool", "a condition Tier 1 does not follow".to_string());
                };
                let (l, r) = (self.term(l, side), self.term(r, side));
                format!("({} {} {})", symbol, l, r)
            }
            Cond::Far(op, far) => {
                let (v, c) = (&far.0, &far.1);
                let Some(symbol) = relation(*op) else {
                    return self.fresh("Bool", "a condition Tier 1 does not follow".to_string());
                };
                let v = self.term(v, side);
                format!("({} {} {})", symbol, v, exact_num(*c))
            }
            Cond::And(a, b) => {
                let (a, b) = (self.cond(a, side), self.cond(b, side));
                format!("(and {} {})", a, b)
            }
            Cond::Or(a, b) => {
                let (a, b) = (self.cond(a, side), self.cond(b, side));
                format!("(or {} {})", a, b)
            }
            Cond::Not(a) => format!("(not {})", self.cond(a, side)),
            Cond::Unknown => self.fresh("Bool", "a condition Tier 1 does not follow".to_string()),
        }
    }

    /// The obligation, denied.
    fn deny(&mut self, goal: &Goal, side: &mut Vec<String>) -> String {
        match goal {
            Goal::Fits(v, Some(t)) => {
                let x = self.term(v, side);
                let (lo, hi) = range(*t);
                format!("(or (< {x} {lo}) (> {x} {hi}))", x = x, lo = lo, hi = hi)
            }
            Goal::Fits(_, None) => "true".to_string(),
            Goal::Index(i, len) => {
                let x = self.term(i, side);
                format!("(or (< {x} 0) (>= {x} {n}))", x = x, n = len)
            }
            Goal::NonZero(d) => format!("(= {} 0)", self.term(d, side)),
            Goal::Shift(k, bits) => {
                let x = self.term(k, side);
                format!("(or (< {x} 0) (>= {x} {w}))", x = x, w = bits)
            }
            Goal::Holds(c) => format!("(not {})", self.cond(c, side)),
        }
    }

    fn text(self, o: &Obligation, total: usize, cases: Vec<(Vec<String>, Vec<String>)>) -> String {
        let fx = self.fx;
        let mut out = String::new();
        let mut what = o.kind.name().to_string();
        if !o.detail.is_empty() {
            what = format!("{} {}", what, o.detail);
        }
        if let Tag::Clause(s) = o.tag {
            if let Some(clause) = fx.src.get(s.start..s.end) {
                what = format!("{}, `{}`", what, one_line(clause));
            }
        }
        let _ = writeln!(
            out,
            "; Tier 1 discharged this obligation of `{}` at {}:{}: {}.",
            fx.syntax.name, o.span.line, o.span.col, what
        );
        out.push_str(
            "; The query holds Tier 1's facts there and denies the obligation: a solver must answer unsat.\n",
        );
        out.push_str("(set-logic QF_LIA)\n");
        // The bindings in scope, each within its type, then the other variables the query uses.
        let mut declared = BTreeSet::new();
        let mut ranges = Vec::new();
        for &b in &fx.scope {
            // The binding's variable, then each field path of it Tier 1 follows.
            let int = match fx.typing.bindings[b] {
                Ty::Int(Int::One(t)) => Some(t),
                _ => None,
            };
            let own = fx.vars[b].map(|v| (v, int));
            let paths = fx.paths[b].iter().map(|p| {
                let t = match &p.shape {
                    super::Shape::Int { int, .. } => *int,
                    _ => None,
                };
                (p.var, t)
            });
            for (v, t) in own.into_iter().chain(paths) {
                if !declared.insert(v) {
                    continue;
                }
                let (name, comment) = &self.names[v];
                let _ = writeln!(out, "(declare-const {} Int) ; {}", name, comment);
                if let Some(t) = t {
                    let (lo, hi) = range(t);
                    ranges.push(format!(
                        "(assert (and (<= {lo} {x}) (<= {x} {hi})))",
                        lo = lo,
                        x = name,
                        hi = hi
                    ));
                }
            }
        }
        let bindings: BTreeSet<usize> = fx.vars.iter().flatten().copied().collect();
        for v in &self.used {
            if declared.insert(*v) {
                let (name, comment) = &self.names[*v];
                let scope = if bindings.contains(v) && Some(*v) != fx.result {
                    ", not in scope here"
                } else {
                    ""
                };
                let _ = writeln!(out, "(declare-const {} Int) ; {}{}", name, comment, scope);
            }
        }
        for (name, sort, comment) in &self.fresh {
            let _ = writeln!(out, "(declare-const {} {}) ; {}", name, sort, comment);
        }
        if !ranges.is_empty() {
            out.push_str("; The type of each binding in scope.\n");
            for r in ranges {
                out.push_str(&r);
                out.push('\n');
            }
        }
        let unreached = total - cases.len();
        match cases.as_slice() {
            [(facts, denial)] => {
                if unreached > 0 {
                    let _ = writeln!(
                        out,
                        "; Tier 1 decided it on {} states; no path reaches the other {}.",
                        total, unreached
                    );
                }
                out.push_str("; Tier 1's state: its interval and octagon constraints.\n");
                for f in facts {
                    let _ = writeln!(out, "(assert {})", f);
                }
                out.push_str("; The obligation, denied.\n");
                for d in denial {
                    let _ = writeln!(out, "(assert {})", d);
                }
            }
            _ => {
                let _ = writeln!(
                    out,
                    "; The obligation, denied in each of the {} states Tier 1 decided it on, with that state's constraints.",
                    cases.len()
                );
                if unreached > 0 {
                    let _ = writeln!(
                        out,
                        "; No path reaches the other {} of the {} states.",
                        unreached, total
                    );
                }
                out.push_str("(assert (or\n");
                for (facts, denial) in &cases {
                    out.push_str("  (and\n");
                    for line in facts.iter().chain(denial) {
                        let _ = writeln!(out, "    {}", line);
                    }
                    out.push_str("  )\n");
                }
                out.push_str("))\n");
            }
        }
        out.push_str("(check-sat)\n(exit)\n");
        out
    }
}

/// The symbol of each octagon variable, and what it stands for: a binding by its name, any other
/// as `|@k|`, `k` its index in the octagon.
fn names(fx: &Fx) -> Vec<(String, String)> {
    let tree = fx.tree;
    let at = |span: Span| format!("{}:{}", span.line, span.col);
    let mut out: Vec<(String, String)> = (0..fx.count)
        .map(|v| (format!("|@{}|", v), "a value Tier 1 follows".to_string()))
        .collect();
    let mut name = |v: usize, what: String| {
        if let Some(slot) = out.get_mut(v) {
            slot.1 = what;
        }
    };
    for (i, site) in fx.sites.iter().enumerate() {
        let e = ExprId(i as u32);
        let place = at(tree.expr(e).span);
        match site {
            Site::Value(v) => name(
                *v,
                format!(
                    "the value of `{}` at {}",
                    one_line(&fx.own_text(e).0),
                    place
                ),
            ),
            Site::Call(c) => {
                let Some(callee) = fx.callees.get(c.callee).copied().flatten() else {
                    continue;
                };
                let f = &callee.syntax.name;
                for (k, p) in callee.syntax.params.iter().enumerate() {
                    if let Some(v) = c.ins.get(k).copied().flatten() {
                        let what =
                            format!("argument `{}` of the call to `{}` at {}", p.name, f, place);
                        name(v, what);
                    }
                    if let Some(v) = c.outs.get(k).copied().flatten() {
                        let what = format!(
                            "what the call to `{}` at {} leaves in `{}`",
                            f, place, p.name
                        );
                        name(v, what);
                    }
                }
                if let Some(v) = c.result {
                    name(v, format!("the result of the call to `{}` at {}", f, place));
                }
            }
            Site::None => {}
        }
    }
    for t in fx.temps {
        name(t, "a value of Tier 1's comparisons".to_string());
    }
    name(fx.scratch, "Tier 1's scratch value".to_string());
    name(fx.probe, "the value a diagnostic names".to_string());
    for (s, measures) in fx.measures.iter().enumerate() {
        let StmtKind::While { decreases, .. } = &tree.stmts[s].kind else {
            continue;
        };
        for (e, (start, end)) in decreases.iter().zip(measures) {
            let text = one_line(&fx.own_text(*e).0);
            name(
                *start,
                format!(
                    "the measure `{}` at the start of a round, or its change since",
                    text
                ),
            );
            name(
                *end,
                format!("the change of the measure `{}` in a round", text),
            );
        }
    }
    let clauses = fx
        .syntax
        .contracts
        .iter()
        .filter(|c| c.kind == ContractKind::Decreases);
    for (c, (entry, call)) in clauses.zip(&fx.entry_measures) {
        let text = one_line(&fx.own_text(c.expr).0);
        name(*entry, format!("the measure `{}` on entry", text));
        name(*call, format!("the measure `{}` at a recursive call", text));
    }
    if let Some(r) = fx.result {
        name(r, format!("the result of `{}`", fx.syntax.name));
        if let Some(slot) = out.get_mut(r) {
            slot.0 = "|result|".to_string();
        }
    }
    let mut taken: BTreeSet<String> = BTreeSet::new();
    // Each field path Tier 1 follows of a binding of a struct type, as it is written.
    for (b, binding) in fx.resolution.bindings.iter().enumerate() {
        for p in &fx.paths[b] {
            let text = format!("{}.{}", binding.name, p.names.join("."));
            let mut symbol = format!("|{}|", text);
            if !taken.insert(symbol.clone()) {
                symbol = format!("|{}.{}|", text, p.var);
            }
            if let Some(slot) = out.get_mut(p.var) {
                *slot = (symbol, format!("`{}`, a field of `{}`", text, binding.name));
            }
        }
    }
    for (b, binding) in fx.resolution.bindings.iter().enumerate() {
        let Some(v) = fx.vars[b] else {
            continue;
        };
        if binding.kind == BindingKind::Result {
            continue;
        }
        let mut symbol = format!("|{}|", binding.name);
        if symbol == "|result|" || !taken.insert(symbol.clone()) {
            symbol = format!("|{}.{}|", binding.name, v);
        }
        if let Some(slot) = out.get_mut(v) {
            *slot = (
                symbol,
                format!(
                    "`{}`, a `{}`",
                    binding.name,
                    fx.typing.bindings[b].spelling()
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::tier1::ObligationKind;

    /// A discharged obligation's kind and `must`, and its query.
    type Listed = (ObligationKind, bool, Option<String>);

    /// Each function's obligations and queries, as `mn smt` gets them.
    fn queries(src: &str) -> Vec<(String, Vec<Listed>)> {
        let text = format!("module t\n{}\n", src);
        crate::tier1_queries(&text)
            .into_iter()
            .map(|(name, report, queries)| {
                let proven = report.obligations.iter().filter(|o| o.proven).count();
                assert_eq!(
                    queries.len(),
                    proven,
                    "a query for each discharged obligation"
                );
                let mut indexes: Vec<usize> = queries.iter().map(|q| q.obligation).collect();
                indexes.dedup();
                assert_eq!(indexes.len(), proven);
                let listed = queries
                    .into_iter()
                    .map(|q| {
                        let o = &report.obligations[q.obligation];
                        assert!(o.proven);
                        if let Some(t) = &q.text {
                            well_formed(t);
                        }
                        (o.kind, o.must, q.text)
                    })
                    .collect();
                (name, listed)
            })
            .collect()
    }

    /// The query of the only obligation of `kind` in the module's last function.
    fn only(src: &str, kind: ObligationKind) -> String {
        let all = queries(src);
        let found: Vec<String> = all
            .last()
            .map(|(_, qs)| {
                qs.iter()
                    .filter(|q| q.0 == kind)
                    .filter_map(|q| q.2.clone())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(found.len(), 1, "{:?} in {}", kind, src);
        found[0].clone()
    }

    /// The query of the only obligation of `kind` in the module's function `name` whose text
    /// holds `needle`.
    fn pick(src: &str, name: &str, kind: ObligationKind, needle: &str) -> String {
        let found: Vec<String> = queries(src)
            .into_iter()
            .filter(|(n, _)| n == name)
            .flat_map(|(_, qs)| qs)
            .filter(|q| q.0 == kind)
            .filter_map(|q| q.2)
            .filter(|t| t.contains(needle))
            .collect();
        assert_eq!(
            found.len(),
            1,
            "{:?} `{}` of {} in {}",
            kind,
            needle,
            name,
            src
        );
        found[0].clone()
    }

    /// The query's text is SMT-LIB 2 as a solver reads it: balanced parentheses, the logic set
    /// first, every symbol declared once before it is used, and one `check-sat`.
    fn well_formed(text: &str) {
        let mut depth = 0i64;
        let mut quoted = false;
        let mut comment = false;
        for ch in text.chars() {
            match (comment, quoted, ch) {
                (true, _, '\n') => comment = false,
                (true, _, _) => {}
                (false, false, ';') => comment = true,
                (false, _, '|') => quoted = !quoted,
                (false, false, '(') => depth += 1,
                (false, false, ')') => {
                    depth -= 1;
                    assert!(depth >= 0, "{}", text);
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "{}", text);
        assert!(!quoted, "{}", text);
        let code: Vec<&str> = text
            .lines()
            .map(|l| l.split(" ; ").next().unwrap_or(""))
            .filter(|l| !l.trim_start().starts_with(';'))
            .collect();
        assert_eq!(code.first(), Some(&"(set-logic QF_LIA)"), "{}", text);
        assert_eq!(code.iter().filter(|l| **l == "(check-sat)").count(), 1);
        assert_eq!(code.last(), Some(&"(exit)"));
        let mut declared = std::collections::BTreeSet::new();
        for line in &code {
            let symbols: Vec<&str> = line.split('|').skip(1).step_by(2).collect();
            if let Some(rest) = line.strip_prefix("(declare-const |") {
                let name = rest.split('|').next().unwrap_or("");
                assert!(
                    declared.insert(name.to_string()),
                    "{} twice: {}",
                    name,
                    text
                );
                continue;
            }
            for s in symbols {
                assert!(declared.contains(s), "`{}` used undeclared: {}", s, text);
            }
        }
    }

    /// The query of an index Tier 1 proves from a `requires`, as written in full: the binding in
    /// scope within `usize`, Tier 1's bounds on it, and the index outside the array. If the
    /// export wrote a constraint, a bound or the denial wrongly, it would differ.
    #[test]
    fn test_each_discharged_obligation_is_written_as_an_smt_query() {
        let src = "pub fn pick(xs: [4]u8, i: usize) -> u8\n  requires i < 4\n= xs[i]";
        assert_eq!(
            only(src, ObligationKind::Index),
            "\
; Tier 1 discharged this obligation of `pick` at 4:6: index into an array of 4.
; The query holds Tier 1's facts there and denies the obligation: a solver must answer unsat.
(set-logic QF_LIA)
(declare-const |i| Int) ; `i`, a `usize`
; The type of each binding in scope.
(assert (and (<= 0 |i|) (<= |i| 18446744073709551615)))
; Tier 1's state: its interval and octagon constraints.
(assert (<= (+ (- |i|) (- |i|)) 0))
(assert (<= (+ |i| |i|) 6))
; The obligation, denied.
(assert (or (< |i| 0) (>= |i| 4)))
(check-sat)
(exit)
"
        );
        // An obligation Tier 1 does not discharge has no query, and one no path reaches has
        // one without text.
        let unproven = queries("pub fn get(xs: [4]u8, i: usize) -> u8 = xs[i]");
        assert!(unproven[0].1.iter().all(|q| q.0 != ObligationKind::Index));
        let after_todo = queries("pub fn add_todo(x: u8) -> u8 = x + todo(\"later\")");
        assert!(after_todo[0]
            .1
            .iter()
            .any(|q| q.0 == ObligationKind::Range && q.2.is_none()));
        // Each kind denies what it states.
        let refined = only(
            "pub fn lit(i: usize) -> usize[0..=9] {\n  guard i <= 9 else {\n    todo(\"no\")\n  }\n  i\n}",
            ObligationKind::Refinement,
        );
        assert!(
            refined.contains("(assert (not (and (>= |i| 0) (<= |i| 9))))"),
            "{}",
            refined
        );
        let narrowed = only(
            "pub fn small(x: u16) -> u8 {\n  guard x < 200 else {\n    todo(\"no\")\n  }\n  x\n}",
            ObligationKind::Range,
        );
        assert!(
            narrowed.contains("(assert (or (< |x| 0) (> |x| 255)))"),
            "{}",
            narrowed
        );
        let divided = only(
            "pub fn div(a: i32, b: i32) -> i32 {\n  guard b > 0 else {\n    todo(\"no\")\n  }\n  a / b\n}",
            ObligationKind::Divisor,
        );
        assert!(divided.contains("(assert (= |b| 0))"), "{}", divided);
        let shifted = only(
            "pub fn shl(x: u8, k: u8) -> u16 {\n  guard k < 8 else {\n    todo(\"no\")\n  }\n  x << k\n}",
            ObligationKind::Shift,
        );
        assert!(
            shifted.contains("(assert (or (< |k| 0) (>= |k| 8)))"),
            "{}",
            shifted
        );
        let called = only(
            "fn half(x: u8) -> u8\n  requires x > 3\n= x / 2\npub fn call(x: u8) -> u8 {\n  guard x > 5 else {\n    todo(\"no\")\n  }\n  half(x)\n}",
            ObligationKind::Requires,
        );
        assert!(
            called.contains("argument `x` of the call to `half`"),
            "{}",
            called
        );
        assert!(called.contains("(assert (not (> |@"), "{}", called);
        let ensured = only(
            "pub fn up(x: u8) -> u8\n  ensures result > x\n{\n  guard x < 10 else {\n    todo(\"no\")\n  }\n  x + 1\n}",
            ObligationKind::Ensures,
        );
        assert!(
            ensured.contains("(assert (not (> |result| |x|)))"),
            "{}",
            ensured
        );
        // A quotient by a constant is a fresh value with the facts of division toward zero.
        let halved = only(
            "pub fn half(x: i8) -> i8[-64..=63] = x / 2",
            ObligationKind::Refinement,
        );
        assert!(
            halved.contains("(=> (< |x| 0) (and (< (- (* 2 |?0|) 2) |x|) (<= |x| (* 2 |?0|))))"),
            "{}",
            halved
        );
        // Added to nothing, the quotient stands alone; each mention is a fresh constant.
        assert!(
            halved.contains("(assert (not (and (>= |?0| (- 64)) (<= |?1| 63))))"),
            "{}",
            halved
        );
    }

    /// `requires a < b` at a call and `ensures result == x`, written in full: the header names
    /// the clause, a call's arguments and the function's result are values Tier 1 follows and no
    /// bindings out of scope, and each comparison keeps its relation.
    #[test]
    fn test_queries_of_contract_clauses_are_written_in_full() {
        let called = only(
            "fn take(a: u8, b: u8) -> u8\n  requires a < b\n= b\npub fn call(x: u8) -> u8 {\n  guard x < 9 else {\n    todo(\"no\")\n  }\n  take(x, 9)\n}",
            ObligationKind::Requires,
        );
        assert_eq!(
            called,
            "\
; Tier 1 discharged this obligation of `call` at 9:3: requires, `a < b`.
; The query holds Tier 1's facts there and denies the obligation: a solver must answer unsat.
(set-logic QF_LIA)
(declare-const |x| Int) ; `x`, a `u8`
(declare-const |@2| Int) ; argument `a` of the call to `take` at 9:3
(declare-const |@3| Int) ; argument `b` of the call to `take` at 9:3
; The type of each binding in scope.
(assert (and (<= 0 |x|) (<= |x| 255)))
; Tier 1's state: its interval and octagon constraints.
(assert (<= (+ (- |x|) (- |x|)) 0))
(assert (<= (+ |@2| (- |x|)) 0))
(assert (<= (+ (- |@2|) (- |x|)) 0))
(assert (<= (+ |@3| (- |x|)) 9))
(assert (<= (+ (- |@3|) (- |x|)) (- 9)))
(assert (<= (+ |x| |x|) 16))
(assert (<= (+ |@2| |x|) 16))
(assert (<= (+ (- |@2|) |x|) 0))
(assert (<= (+ |@3| |x|) 17))
(assert (<= (+ (- |@3|) |x|) (- 1)))
(assert (<= (+ (- |@2|) (- |@2|)) 0))
(assert (<= (+ |@3| (- |@2|)) 9))
(assert (<= (+ (- |@3|) (- |@2|)) (- 9)))
(assert (<= (+ |@2| |@2|) 16))
(assert (<= (+ |@3| |@2|) 17))
(assert (<= (+ (- |@3|) |@2|) (- 1)))
(assert (<= (+ (- |@3|) (- |@3|)) (- 18)))
(assert (<= (+ |@3| |@3|) 18))
; The obligation, denied.
(assert (not (< |@2| |@3|)))
(check-sat)
(exit)
"
        );
        let ensured = only(
            "pub fn same(x: u8) -> u8\n  ensures result == x\n= x",
            ObligationKind::Ensures,
        );
        assert_eq!(
            ensured,
            "\
; Tier 1 discharged this obligation of `same` at 4:3: ensures, `result == x`.
; The query holds Tier 1's facts there and denies the obligation: a solver must answer unsat.
(set-logic QF_LIA)
(declare-const |x| Int) ; `x`, a `u8`
(declare-const |result| Int) ; the result of `same`
; The type of each binding in scope.
(assert (and (<= 0 |x|) (<= |x| 255)))
; Tier 1's state: its interval and octagon constraints.
(assert (<= (+ (- |result|) (- |result|)) 0))
(assert (<= (+ |x| (- |result|)) 0))
(assert (<= (+ (- |x|) (- |result|)) 0))
(assert (<= (+ |result| |result|) 510))
(assert (<= (+ |x| |result|) 510))
(assert (<= (+ (- |x|) |result|) 0))
(assert (<= (+ (- |x|) (- |x|)) 0))
(assert (<= (+ |x| |x|) 510))
; The obligation, denied.
(assert (not (= |result| |x|)))
(check-sat)
(exit)
"
        );
    }

    /// Each comparison has its SMT-LIB relation, `!=` too, though no clause in the Tier-1
    /// fragment uses it; any other operator has none, and a condition on it is not followed.
    #[test]
    fn test_each_comparison_has_its_relation() {
        use mente_syntax::tree::BinaryOp::{Add, And, Eq, Ge, Gt, Le, Lt, Ne};
        assert_eq!(
            [Lt, Le, Gt, Ge, Eq, Ne].map(super::relation),
            ["<", "<=", ">", ">=", "=", "distinct"].map(Some)
        );
        assert_eq!([Add, And].map(super::relation), [None, None]);
    }

    /// A value Tier 1 knows only by its bounds is a fresh constant between them, with no bound
    /// where it has none; a quotient added to a value keeps the value.
    #[test]
    fn test_values_known_by_bounds_are_fresh_constants_between_them() {
        let both = only("pub fn tri(x: u8) -> u16 = x * 3", ObligationKind::Range);
        assert!(
            both.contains("(declare-const |?0| Int) ; a value Tier 1 knows from 0 to 765\n"),
            "{}",
            both
        );
        assert!(
            both.contains("; The obligation, denied.\n(assert (<= 0 |?0|))\n(assert (<= |?0| 765))\n(assert (or (< |?0| 0) (> |?0| 65535)))\n"),
            "{}",
            both
        );
        let above = only(
            "pub fn up(a: u128, x: u128) -> u128 {\n  guard x > 0 else {\n    todo(\"no\")\n  }\n  a / (x * 2)\n}",
            ObligationKind::Divisor,
        );
        assert!(
            above.contains("; a value Tier 1 knows from 2 to unbounded\n"),
            "{}",
            above
        );
        assert!(
            above
                .contains("; The obligation, denied.\n(assert (<= 2 |?0|))\n(assert (= |?0| 0))\n"),
            "{}",
            above
        );
        let below = only(
            "pub fn down(a: i128, y: i128) -> i128 {\n  guard y < 0 else {\n    todo(\"no\")\n  }\n  a / (y * 2)\n}",
            ObligationKind::Divisor,
        );
        assert!(
            below.contains("; a value Tier 1 knows from unbounded to -2\n"),
            "{}",
            below
        );
        assert!(
            below.contains(
                "; The obligation, denied.\n(assert (<= |?0| (- 2)))\n(assert (= |?0| 0))\n"
            ),
            "{}",
            below
        );
        let middle = only(
            "pub fn mid(lo: u8, hi: u8) -> u8 {\n  guard lo <= hi else {\n    todo(\"no\")\n  }\n  lo + (hi - lo) / 2\n}",
            ObligationKind::Range,
        );
        assert!(
            middle.contains("(assert (or (< (+ |lo| |?0|) 0) (> (+ |lo| |?0|) 255)))"),
            "{}",
            middle
        );
        assert!(
            middle.contains("(<= (* 2 |?0|) (+ |hi| (- |lo|)))"),
            "{}",
            middle
        );
    }

    /// Of the states an obligation was decided on, those no path reaches are left out and
    /// counted; an obligation no path reaches has no query, unless its state's own constraints
    /// contradict each other.
    #[test]
    fn test_states_no_path_reaches_are_left_out() {
        let one = "pub fn one(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    if i == 200 {\n      return i\n    } else {\n      i += 1\n    }\n  }\n  i\n}";
        let smaller = pick(one, "one", ObligationKind::Decreases, "decreases.");
        assert!(
            smaller.contains(
                "; Tier 1 decided it on 2 states; no path reaches the other 1.\n; Tier 1's state"
            ),
            "{}",
            smaller
        );
        let two = "pub fn two(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    if i == 200 {\n      return i\n    } else if i < 100 {\n      i += 1\n    } else {\n      i += 1\n    }\n  }\n  i\n}";
        let smaller = pick(two, "two", ObligationKind::Decreases, "decreases.");
        assert!(
            smaller.contains("in each of the 2 states Tier 1 decided it on, with that state's constraints.\n; No path reaches the other 1 of the 3 states.\n(assert (or\n"),
            "{}",
            smaller
        );
        // After `todo`, among constraints that hold a negative constant, no contradiction.
        let late = queries(
            "pub fn late(x: u8) -> u8 {\n  guard x > 0 else {\n    todo(\"no\")\n  }\n  x + todo(\"later\")\n}",
        );
        assert!(
            late[0]
                .1
                .iter()
                .any(|q| q.0 == ObligationKind::Range && q.2.is_none()),
            "{:?}",
            late
        );
        // Under `x > 5` with `x < 3`, the state's constraints contradict each other, once.
        let never = only(
            "pub fn never(xs: [4]u8, x: u8) -> bool {\n  guard x < 3 else {\n    todo(\"no\")\n  }\n  x > 5 and xs[x] > 0\n}",
            ObligationKind::Index,
        );
        assert_eq!(never.matches("(assert (<= 0 (- ").count(), 1, "{}", never);
    }

    /// A variable is named after its binding, a binding called `result` apart from the function's
    /// result; one out of scope is said to be, and a measure is described by its clause.
    #[test]
    fn test_variables_are_named_and_described() {
        let shadow = only(
            "pub fn g(x: u8) -> u8\n  ensures result == x\n{\n  val result: u8 = x\n  result\n}",
            ObligationKind::Ensures,
        );
        assert!(
            shadow.contains("(declare-const |result| Int) ; the result of `g`\n"),
            "{}",
            shadow
        );
        // Out of scope at the `ensures`, after the block; declared once, as `well_formed` checks.
        assert!(
            shadow.contains("(declare-const |result.")
                && shadow.contains("Int) ; `result`, a `u8`, not in scope here\n"),
            "{}",
            shadow
        );
        let step = only(
            "pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i <= n {\n    val step: u8 = 1\n    i += step\n  }\n  i\n}",
            ObligationKind::Invariant,
        );
        assert!(
            step.contains("(declare-const |step| Int) ; `step`, a `u8`, not in scope here\n"),
            "{}",
            step
        );
        let measured = pick(
            "pub fn sum(n: u8) -> u16\n  requires n < 100\n  decreases n\n{\n  guard n > 0 else {\n    return 0\n  }\n  val m: u8 = n - 1\n  m + sum(m)\n}",
            "sum",
            ObligationKind::Decreases,
            "decreases.",
        );
        assert!(
            measured.contains("Int) ; the measure `n` on entry\n"),
            "{}",
            measured
        );
    }

    /// A narrowing into `u128` from a signed source: the query denies it against the type's
    /// whole range, from 0 to 2^128 - 1, and Tier 1 discharges it only where the value cannot be
    /// negative, so that no query of it is satisfiable.
    #[test]
    fn test_a_narrowing_into_u128_is_denied_against_the_whole_type() {
        let guarded = only(
            "pub fn widen(x: i8) -> u128 {\n  guard x >= 0 else {\n    todo(\"no\")\n  }\n  x\n}",
            ObligationKind::Range,
        );
        assert!(
            guarded.contains(
                "(assert (or (< |x| 0) (> |x| 340282366920938463463374607431768211455)))"
            ),
            "{}",
            guarded
        );
        assert!(
            guarded.contains("(assert (<= (+ (- |x|) (- |x|)) 0))"),
            "the guard's fact `x >= 0`: {}",
            guarded
        );
        // Without the guard `x` may be -1, which `u128` cannot hold: a run-time check, no query.
        let unguarded = queries("pub fn widen(x: i8) -> u128 = x");
        assert!(
            unguarded[0].1.iter().all(|q| q.0 != ObligationKind::Range),
            "{:?}",
            unguarded
        );
    }

    /// A field path Tier 1 follows is a variable of the query, named as written and bounded by
    /// its type when its binding is in scope.
    #[test]
    fn test_field_paths_are_named_and_bounded() {
        let q = only(
            "pub struct P {\n  a: u8\n  b: i8\n}\npub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  p.a\n}",
            ObligationKind::Refinement,
        );
        assert!(
            q.contains("(declare-const |p.a| Int) ; `p.a`, a field of `p`"),
            "{}",
            q
        );
        assert!(
            q.contains("(assert (and (<= 0 |p.a|) (<= |p.a| 255)))"),
            "{}",
            q
        );
        assert!(
            q.contains("(assert (and (<= (- 128) |p.b|) (<= |p.b| 127)))"),
            "{}",
            q
        );
    }

    /// A refinement at a 128-bit type's limit is denied against its exact bound, and the
    /// bounds the state keeps beyond the octagon's reach are among the query's facts.
    #[test]
    fn test_bounds_beyond_the_octagon_are_written_exactly() {
        let q = only(
            "pub fn take(x: u128[0..340282366920938463463374607431768211455]) -> u128 = x\npub fn f(y: u128[0..340282366920938463463374607431768211455]) -> u128 = take(y)",
            ObligationKind::Refinement,
        );
        assert!(
            q.contains("340282366920938463463374607431768211454)"),
            "the bound kept beyond the octagon: {}",
            q
        );
        assert!(
            q.contains("(< |x| 340282366920938463463374607431768211455)")
                || q.contains("340282366920938463463374607431768211455)))"),
            "the refinement denied exactly: {}",
            q
        );
        let i = only(
            "pub fn take(x: i128[-170141183460469231731687303715884105728..=0]) -> i128 = x\npub fn f() -> i128 = take(-170141183460469231731687303715884105728)",
            ObligationKind::Refinement,
        );
        assert!(
            i.contains("(- 170141183460469231731687303715884105728)"),
            "a negative bound as a numeral: {}",
            i
        );
    }

    /// A loop's obligations decided on several states deny the obligation in each of them.
    #[test]
    fn test_obligations_of_several_states_are_denied_in_each() {
        let src = "pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i <= n decreases n - i {\n    i += 1\n  }\n  i\n}";
        let all = queries(src);
        let qs = &all[0].1;
        let invariant: Vec<&String> = qs
            .iter()
            .filter(|q| q.0 == ObligationKind::Invariant)
            .filter_map(|q| q.2.as_ref())
            .collect();
        assert_eq!(invariant.len(), 1);
        assert!(
            invariant[0].contains("in each of the 2 states"),
            "{}",
            invariant[0]
        );
        assert_eq!(invariant[0].matches("(not (<= |i| |n|))").count(), 2);
        assert!(
            !invariant[0].contains("No path reaches"),
            "{}",
            invariant[0]
        );
        let measures: Vec<&String> = qs
            .iter()
            .filter(|q| q.0 == ObligationKind::Decreases)
            .filter_map(|q| q.2.as_ref())
            .collect();
        assert_eq!(measures.len(), 2, "never negative, and smaller");
        assert!(measures.iter().any(|m| m.contains("(not (>= ")));
        let recursive = queries(
            "pub fn sum(n: u8) -> u16\n  decreases n\n{\n  guard n > 0 else {\n    return 0\n  }\n  val m: u8 = n - 1\n  m + sum(m)\n}",
        );
        let decreases = recursive[0]
            .1
            .iter()
            .filter(|q| q.0 == ObligationKind::Decreases && q.2.is_some())
            .count();
        assert_eq!(
            decreases, 2,
            "never negative at the call, and smaller than on entry"
        );
    }
}

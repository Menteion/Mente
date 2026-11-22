//! The containment oracle: Tier 1's abstract states checked against the reference interpreter's
//! concrete runs ([`tier1-containment`](../../../docs/design-review.md#tier1-containment)).
//!
//! The comparison of the three paths sees a wrong proof only where a check Tier 1 proved is
//! reached and fails in a run. A state that is too narrow, or empty, through a modelling error
//! proves checks that no run of the comparison happens to break, and from an empty state every
//! obligation counts as proven. So every call the comparison makes runs once more on the
//! reference interpreter, traced, and each point it reaches is held against what Tier 1's
//! recording pass claimed there (`mente_sema::tier1::states`):
//! - before a statement, at the end of a block and at the head of a loop's round, the values of
//!   the bindings in scope, and of the field paths Tier 1 follows, must satisfy one of the states
//!   Tier 1 recorded at that point: the pass may reach a point more than once, and the states
//!   count together. An empty state there, or none at all, which is what Tier 1's walk leaves at
//!   a point it holds unreachable, is a problem;
//! - each value an expression gives must lie within one of the values Tier 1 recorded for it, and
//!   none of them may be from an empty state. An expression Tier 1 recorded no value for, such as
//!   a written invariant it evaluates only as Houdini's candidate, is no claim.
//!
//! Tier 1 and the interpreter walk the same typed tree, so a point is the function's index in
//! the module and a statement, block or expression of its tree, and a variable is a binding's
//! slot: nothing is matched by span or by text. The claims are laid out by those indexes, and a
//! point's variables once for all its states, so that an event costs a lookup and a comparison
//! for each constraint, and words are written only for a problem.

use std::collections::HashSet;

use mente_interp::{Event, Int, Limits, Value};
use mente_sema::octagon::Exact;
use mente_sema::tier1::states::{Point, Signed, States};
use mente_sema::CheckedModule;
use mente_syntax::tree::{ExprId, Tree};

use super::{printed, CallArgs, Printed};

/// The most problems one program reports: past them the first ones say enough.
const MOST: usize = 20;

/// A term of a relation: a variable by its position among a point's, and whether it is negated.
type Term = (usize, bool);

/// One state Tier 1 recorded at a point, over the point's variables by their position.
#[derive(Clone, Debug, Default)]
struct Record {
    empty: bool,
    bounds: Vec<(usize, Option<Exact>, Option<Exact>)>,
    /// `a + b <= c`, each term a position and whether it is negated.
    relations: Vec<(Term, Term, i128)>,
    /// The same constraints as `i128`s, for the check every event runs, `None` for no bound;
    /// none when a bound does not fit one.
    fast: Option<Fast>,
}

/// A record's constraints as `i128`s.
#[derive(Clone, Debug, Default)]
struct Fast {
    bounds: Vec<(usize, i128, i128)>,
    relations: Vec<(Term, Term, i128)>,
}

impl Fast {
    fn of(r: &Record) -> Option<Fast> {
        let bounds = r
            .bounds
            .iter()
            .map(|&(k, lo, hi)| {
                let lo = lo.map_or(Some(i128::MIN), Exact::to_i128)?;
                let hi = hi.map_or(Some(i128::MAX), Exact::to_i128)?;
                Some((k, lo, hi))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Fast {
            bounds,
            relations: r.relations.clone(),
        })
    }

    /// Whether the values `nums` satisfy every constraint, `None` where one is past an `i128`,
    /// which [`judge`] then decides exactly.
    fn holds(&self, nums: &[Num]) -> Option<bool> {
        for &(k, lo, hi) in &self.bounds {
            match nums[k] {
                Num::Missing => {}
                Num::Wide => return None,
                Num::Fits(x) => {
                    if x < lo || x > hi {
                        return Some(false);
                    }
                }
            }
        }
        for &((a, na), (b, nb), c) in &self.relations {
            let (x, y) = match (nums[a], nums[b]) {
                (Num::Fits(x), Num::Fits(y)) => (x, y),
                (Num::Missing, _) | (_, Num::Missing) => continue,
                (Num::Wide, _) | (_, Num::Wide) => return None,
            };
            let x = if na { x.checked_neg()? } else { x };
            let y = if nb { y.checked_neg()? } else { y };
            if x.checked_add(y)? > c {
                return Some(false);
            }
        }
        Some(true)
    }
}

/// A value of a variable, as the fast check reads it.
#[derive(Clone, Copy, Debug)]
enum Num {
    /// The interpreter holds none: its binding is not bound, or not an integer there.
    Missing,
    Fits(i128),
    /// Past an `i128`: the exact check reads it.
    Wide,
}

/// `x` as an `i128`, when it fits one.
fn num(x: Int) -> Option<i128> {
    if x.is_negative() {
        0i128.checked_sub_unsigned(x.magnitude())
    } else {
        i128::try_from(x.magnitude()).ok()
    }
}

/// What Tier 1 claimed at one point: the tracked variables in scope there, each with its binding,
/// its field path and its variable in Tier 1's octagon, and every state it recorded there.
#[derive(Clone, Debug, Default)]
struct AtPoint {
    vars: Vec<(usize, Vec<String>, usize)>,
    records: Vec<Record>,
    /// Whether some state recorded here bounds nothing and relates nothing, so that no value
    /// breaks it: such a point is checked by its lookup alone.
    free: bool,
}

impl AtPoint {
    /// The position of Tier 1's variable `var`, `binding`'s at `path`, added when new.
    fn position(&mut self, var: usize, tracked: &[Option<(usize, Vec<String>)>]) -> Option<usize> {
        if let Some(k) = self.vars.iter().position(|(_, _, v)| *v == var) {
            return Some(k);
        }
        let (binding, path) = tracked.get(var)?.clone()?;
        self.vars.push((binding, path, var));
        Some(self.vars.len() - 1)
    }
}

/// A value Tier 1 claimed for an expression: whether its state was empty, and its bounds.
type Claimed = (bool, Option<Exact>, Option<Exact>);

/// What Tier 1 claimed of an expression's values, reduced to what a value can break.
#[derive(Clone, Debug, Default)]
enum ValueClaim {
    /// No record: no claim.
    #[default]
    Unclaimed,
    /// Some record bounds nothing: no value breaks it.
    Free,
    /// Every record is of an empty state: any value given breaks it.
    Empty,
    /// The bounds of each record of a state that is not empty, and the same as `i128`s, when
    /// they fit, for the check every event runs.
    Within(
        Vec<(Option<Exact>, Option<Exact>)>,
        Option<Vec<(i128, i128)>>,
    ),
}

/// What Tier 1 claimed of one function, laid out by the indexes of its tree.
#[derive(Clone, Debug, Default)]
struct Claims {
    before: Vec<AtPoint>,
    end: Vec<AtPoint>,
    head: Vec<AtPoint>,
    values: Vec<ValueClaim>,
    /// The statements and blocks inside a clause Tier 1 never walks (`States::unwalked`),
    /// where no state is no claim.
    quiet_stmts: Vec<bool>,
    quiet_blocks: Vec<bool>,
}

impl Claims {
    fn of(states: States, tree: &Tree) -> Claims {
        let mut c = Claims {
            before: vec![AtPoint::default(); tree.stmts.len()],
            end: vec![AtPoint::default(); tree.blocks.len()],
            head: vec![AtPoint::default(); tree.stmts.len()],
            values: vec![ValueClaim::Unclaimed; tree.exprs.len()],
            quiet_stmts: vec![false; tree.stmts.len()],
            quiet_blocks: vec![false; tree.blocks.len()],
        };
        for &root in &states.unwalked {
            let sub = tree.subtree(root);
            for s in sub.stmts {
                c.quiet_stmts[s.0 as usize] = true;
            }
            for b in sub.blocks {
                c.quiet_blocks[b.0 as usize] = true;
            }
        }
        let most = states.tracked.iter().map(|t| t.var + 1).max().unwrap_or(0);
        let mut tracked: Vec<Option<(usize, Vec<String>)>> = vec![None; most];
        for t in states.tracked {
            tracked[t.var] = Some((t.binding, t.path));
        }
        for (p, facts) in states.points {
            let Some(at) = c.at_mut(p) else {
                continue;
            };
            let mut r = Record {
                empty: facts.empty,
                ..Record::default()
            };
            for (var, lo, hi) in facts.bounds {
                if let Some(k) = at.position(var, &tracked) {
                    r.bounds.push((k, lo, hi));
                }
            }
            let mut term = |s: Signed| at.position(s.var, &tracked).map(|k| (k, s.neg));
            for (a, b, bound) in facts.relations {
                if let (Some(a), Some(b)) = (term(a), term(b)) {
                    r.relations.push((a, b, bound));
                }
            }
            at.records.push(r);
        }
        let mut values: Vec<Vec<Claimed>> = vec![Vec::new(); tree.exprs.len()];
        for v in states.values {
            if let Some(list) = values.get_mut(v.expr.0 as usize) {
                list.push((v.empty, v.lo, v.hi));
            }
        }
        for (claim, records) in c.values.iter_mut().zip(values) {
            let live: Vec<(Option<Exact>, Option<Exact>)> = records
                .iter()
                .filter(|(empty, _, _)| !*empty)
                .map(|(_, lo, hi)| (*lo, *hi))
                .collect();
            *claim = if records.is_empty() {
                ValueClaim::Unclaimed
            } else if live.is_empty() {
                ValueClaim::Empty
            } else if live.iter().any(|(lo, hi)| lo.is_none() && hi.is_none()) {
                ValueClaim::Free
            } else {
                let fast = live
                    .iter()
                    .map(|&(lo, hi)| {
                        Some((
                            lo.map_or(Some(i128::MIN), Exact::to_i128)?,
                            hi.map_or(Some(i128::MAX), Exact::to_i128)?,
                        ))
                    })
                    .collect::<Option<Vec<_>>>();
                ValueClaim::Within(live, fast)
            };
        }
        for at in c.before.iter_mut().chain(&mut c.end).chain(&mut c.head) {
            for r in &mut at.records {
                r.fast = Fast::of(r);
            }
        }
        for at in c.before.iter_mut().chain(&mut c.end).chain(&mut c.head) {
            at.free = at.records.iter().any(|r| {
                !r.empty
                    && r.relations.is_empty()
                    && r.bounds
                        .iter()
                        .all(|(_, lo, hi)| lo.is_none() && hi.is_none())
            });
        }
        c
    }

    fn at_mut(&mut self, p: Point) -> Option<&mut AtPoint> {
        match p {
            Point::Before(s) => self.before.get_mut(s.0 as usize),
            Point::End(b) => self.end.get_mut(b.0 as usize),
            Point::Head(s) => self.head.get_mut(s.0 as usize),
        }
    }

    /// Whether `p` lies inside a clause Tier 1 never walks, where it makes no claim.
    fn quiet(&self, p: Point) -> bool {
        let at = |v: &[bool], i: u32| v.get(i as usize).copied().unwrap_or(false);
        match p {
            Point::Before(s) | Point::Head(s) => at(&self.quiet_stmts, s.0),
            Point::End(b) => at(&self.quiet_blocks, b.0),
        }
    }

    fn at(&self, p: Point) -> Option<&AtPoint> {
        match p {
            Point::Before(s) => self.before.get(s.0 as usize),
            Point::End(b) => self.end.get(b.0 as usize),
            Point::Head(s) => self.head.get(s.0 as usize),
        }
    }
}

/// What the oracle found in one program: the points and the values it checked, and each problem
/// with the index of the call that showed it.
#[derive(Debug, Default)]
pub struct Outcome {
    pub points: u64,
    pub values: u64,
    pub problems: Vec<(usize, String)>,
    /// Each call's outcome on the reference interpreter, as `mn run` prints it.
    pub printed: Vec<Printed>,
}

fn exact(x: Int) -> Exact {
    Exact::new(x.is_negative(), x.magnitude())
}

fn within(x: Exact, lo: Option<Exact>, hi: Option<Exact>) -> bool {
    lo.is_none_or(|l| l <= x) && hi.is_none_or(|h| x <= h)
}

/// Whether `a + b <= c` holds of the values `x` of `a`'s variable and `y` of `b`'s, as
/// `a <= c - b`.
fn related((a, x): (bool, Exact), (b, y): (bool, Exact), c: i128) -> bool {
    let term = |neg: bool, v: Exact| if neg { v.negated() } else { v };
    let minus_b = term(b, y).negated();
    match minus_b.plus(c) {
        Some(room) => term(a, x) <= room,
        // `c - b` lies past the range of the 128-bit types, on the side of `-b`, since `c` is a
        // constant of the octagon's matrix, far inside it: above every value, or below.
        None => minus_b > Exact::from_i128(0),
    }
}

fn show_bound(b: Option<Exact>) -> String {
    b.map_or("none".to_string(), |b| b.to_string())
}

/// The value at `path` of a binding's value, when it is an integer.
fn at_path(mut v: &Value, path: &[String]) -> Option<Int> {
    for name in path {
        match v {
            Value::Struct(_, fields) => v = &fields.iter().find(|(n, _)| n == name)?.1,
            Value::Int(_) | Value::Bool(_) | Value::Array(_) | Value::Enum(..) | Value::Unit => {
                return None
            }
        }
    }
    match v {
        Value::Int(x) => Some(*x),
        Value::Bool(_) | Value::Array(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => {
            None
        }
    }
}

/// The first integer of a value that `fits` refuses, an array's elements at every depth
/// included.
fn first_outside(v: &Value, fits: &dyn Fn(Exact) -> bool) -> Option<Exact> {
    match v {
        Value::Int(x) => Some(exact(*x)).filter(|x| !fits(*x)),
        Value::Array(items) => items.iter().find_map(|i| first_outside(i, fits)),
        Value::Bool(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => None,
    }
}

/// Whether every integer of `v` lies within one of `live`, `None` where one is past an
/// `i128`: the check every value runs, which the exact one decides past it.
fn all_within(v: &Value, live: &[(i128, i128)]) -> Option<bool> {
    match v {
        Value::Int(x) => {
            let n = num(*x)?;
            Some(live.iter().any(|&(lo, hi)| lo <= n && n <= hi))
        }
        Value::Array(items) => {
            for i in items {
                if !all_within(i, live)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        Value::Bool(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => Some(true),
    }
}

/// Which records of a point a valuation breaks, and how: `None` when one holds it.
enum Verdict {
    Holds,
    Empty,
    /// The first record's broken bounds, by position, and its broken relations, by index.
    Outside(Vec<usize>, Vec<usize>),
}

fn judge(at: &AtPoint, values: &[Option<Exact>]) -> Verdict {
    let mut first: Option<Verdict> = None;
    for r in &at.records {
        if r.empty {
            first.get_or_insert(Verdict::Empty);
            continue;
        }
        let bounds: Vec<usize> = r
            .bounds
            .iter()
            .enumerate()
            .filter(|(_, (k, lo, hi))| values[*k].is_some_and(|x| !within(x, *lo, *hi)))
            .map(|(i, _)| i)
            .collect();
        let relations: Vec<usize> = r
            .relations
            .iter()
            .enumerate()
            .filter(
                |(_, ((a, na), (b, nb), c))| match (values[*a], values[*b]) {
                    (Some(x), Some(y)) => !related((*na, x), (*nb, y), *c),
                    _ => false,
                },
            )
            .map(|(i, _)| i)
            .collect();
        if bounds.is_empty() && relations.is_empty() {
            return Verdict::Holds;
        }
        if !matches!(first, Some(Verdict::Outside(..))) {
            first = Some(Verdict::Outside(bounds, relations));
        }
    }
    first.unwrap_or(Verdict::Holds)
}

/// The oracle's state while it watches one program's runs.
struct Watch<'a> {
    src: &'a str,
    module: &'a CheckedModule,
    claims: &'a [Option<Claims>],
    call: usize,
    seen: HashSet<(usize, String)>,
    /// The values of a point's variables, reused from event to event, exactly and for the fast
    /// check.
    values: Vec<Option<Exact>>,
    nums: Vec<Num>,
    out: Outcome,
}

impl<'a> Watch<'a> {
    fn new(src: &'a str, module: &'a CheckedModule, claims: &'a [Option<Claims>]) -> Watch<'a> {
        Watch {
            src,
            module,
            claims,
            call: 0,
            seen: HashSet::new(),
            values: Vec::new(),
            nums: Vec::new(),
            out: Outcome::default(),
        }
    }

    fn problem(&mut self, function: usize, at: String, detail: String) {
        if self.out.problems.len() >= MOST || !self.seen.insert((function, at.clone())) {
            return;
        }
        let name = self.module.names.get(function).cloned().unwrap_or_default();
        self.out
            .problems
            .push((self.call, format!("in `{}`, {}: {}", name, at, detail)));
    }

    fn tree(&self, function: usize) -> Option<&'a Tree> {
        match self.module.functions.get(function) {
            Some(Some(f)) => Some(&f.syntax.tree),
            _ => None,
        }
    }

    /// Where `p` is, in words.
    fn place(&self, function: usize, p: Point) -> String {
        let Some(tree) = self.tree(function) else {
            return format!("{:?}", p);
        };
        match p {
            Point::Before(s) => {
                let sp = tree.stmt(s).span;
                format!("before the statement at {}:{}", sp.line, sp.col)
            }
            Point::End(b) => {
                let sp = tree.block(b).span;
                format!("at the end of the block at {}:{}", sp.line, sp.col)
            }
            Point::Head(s) => {
                let sp = tree.stmt(s).span;
                format!(
                    "at the head of a round of the loop at {}:{}",
                    sp.line, sp.col
                )
            }
        }
    }

    /// A tracked variable as the program names it: `x`, `p.a.b`.
    fn var_name(&self, function: usize, (binding, path): (usize, &[String])) -> String {
        let base = match self.module.functions.get(function) {
            Some(Some(f)) => f
                .resolution
                .bindings
                .get(binding)
                .map_or("?".to_string(), |b| b.name.clone()),
            _ => "?".to_string(),
        };
        std::iter::once(base)
            .chain(path.iter().cloned())
            .collect::<Vec<_>>()
            .join(".")
    }

    fn point(&mut self, function: usize, p: Point, slots: &[Option<Value>]) {
        self.out.points += 1;
        let claims = self.claims;
        let Some(Some(claims)) = claims.get(function) else {
            return;
        };
        if claims.quiet(p) {
            return;
        }
        let at = match claims.at(p) {
            Some(at) if at.free => return,
            Some(at) if !at.records.is_empty() => at,
            _ => {
                let place = self.place(function, p);
                self.problem(
                    function,
                    place,
                    "Tier 1 holds no state there, as at a point it holds unreachable, yet the \
                     reference interpreter reaches it"
                        .to_string(),
                );
                return;
            }
        };
        // The interpreter's value of each tracked variable in scope, where it has one; a state
        // the fast check finds holding settles the point.
        let mut nums = std::mem::take(&mut self.nums);
        nums.clear();
        // Plain loops and matches: this runs at every point of every run, in the unit tests'
        // build without optimisation too, where each adaptor and closure is a call.
        for (binding, path, _) in &at.vars {
            let held = match slots.get(*binding) {
                Some(Some(v)) => at_path(v, path),
                Some(None) | None => None,
            };
            nums.push(match held {
                None => Num::Missing,
                Some(x) => match num(x) {
                    Some(n) => Num::Fits(n),
                    None => Num::Wide,
                },
            });
        }
        let mut held = false;
        for r in &at.records {
            if let (false, Some(f)) = (r.empty, &r.fast) {
                if f.holds(&nums) == Some(true) {
                    held = true;
                    break;
                }
            }
        }
        self.nums = nums;
        if held {
            return;
        }
        let mut values = std::mem::take(&mut self.values);
        values.clear();
        values.extend(at.vars.iter().map(|(binding, path, _)| {
            let v = slots.get(*binding)?.as_ref()?;
            at_path(v, path).map(exact)
        }));
        let verdict = judge(at, &values);
        match verdict {
            Verdict::Holds => {}
            Verdict::Empty => {
                let place = self.place(function, p);
                self.problem(
                    function,
                    place,
                    "Tier 1's state there is empty, yet the reference interpreter reaches it"
                        .to_string(),
                );
            }
            Verdict::Outside(bounds, relations) => {
                let first = at.records.iter().find(|r| !r.empty);
                let mut broken = Vec::new();
                let name = |k: usize| {
                    let (b, path, _) = &at.vars[k];
                    self.var_name(function, (*b, path))
                };
                let shown = |k: usize| values[k].map_or("?".to_string(), |x| x.to_string());
                if let Some(r) = first {
                    for i in bounds {
                        let (k, lo, hi) = r.bounds[i];
                        broken.push(format!(
                            "`{}` is {}, outside [{}, {}]",
                            name(k),
                            shown(k),
                            show_bound(lo),
                            show_bound(hi)
                        ));
                    }
                    for i in relations {
                        let ((a, na), (b, nb), c) = r.relations[i];
                        let sign = |neg: bool| if neg { "-" } else { "" };
                        broken.push(format!(
                            "`{}` is {} and `{}` is {}, against {}{} + {}{} <= {}",
                            name(a),
                            shown(a),
                            name(b),
                            shown(b),
                            sign(na),
                            name(a),
                            sign(nb),
                            name(b),
                            c
                        ));
                    }
                }
                let others = at.records.len() - 1;
                let detail = format!(
                    "the reference interpreter's values lie outside Tier 1's state there{}: {}",
                    if others > 0 {
                        format!(" (and the {} other states recorded there)", others)
                    } else {
                        String::new()
                    },
                    broken.join("; ")
                );
                let place = self.place(function, p);
                self.problem(function, place, detail);
            }
        }
        self.values = values;
    }

    fn value(&mut self, function: usize, expr: ExprId, v: &Value) {
        self.out.values += 1;
        let claims = self.claims;
        let Some(Some(claims)) = claims.get(function) else {
            return;
        };
        let live = match claims.values.get(expr.0 as usize) {
            None | Some(ValueClaim::Unclaimed | ValueClaim::Free) => return,
            Some(ValueClaim::Empty) => None,
            Some(ValueClaim::Within(live, fast)) => {
                if fast.as_ref().and_then(|f| all_within(v, f)) == Some(true) {
                    return;
                }
                Some(live)
            }
        };
        let outside = match live {
            None => None,
            Some(live) => {
                match first_outside(v, &|x| live.iter().any(|(lo, hi)| within(x, *lo, *hi))) {
                    None => return,
                    Some(x) => Some((x, live)),
                }
            }
        };
        let at = self.expr_place(function, expr);
        let detail = match outside {
            None => "Tier 1 evaluated it in an empty state, yet the reference interpreter gives it"
                .to_string(),
            Some((x, live)) => {
                let claimed: Vec<String> = live
                    .iter()
                    .map(|(lo, hi)| format!("[{}, {}]", show_bound(*lo), show_bound(*hi)))
                    .collect();
                format!(
                    "the reference interpreter gives {}, outside Tier 1's {}",
                    x,
                    claimed.join(" or ")
                )
            }
        };
        self.problem(function, at, detail);
    }

    /// An expression in words, by its text: "the value of `a % n` at 5:12".
    fn expr_place(&self, function: usize, expr: ExprId) -> String {
        let Some(tree) = self.tree(function) else {
            return format!("{:?}", expr);
        };
        let sp = tree.expr(expr).span;
        let text = self.src.get(sp.start..sp.end).unwrap_or("?");
        let text = match text.char_indices().nth(60) {
            Some((cut, _)) => format!("{}...", &text[..cut]),
            None => text.to_string(),
        };
        format!("the value of `{}` at {}:{}", text, sp.line, sp.col)
    }

    fn event(&mut self, e: Event<'_>) {
        match e {
            Event::Before {
                function,
                stmt,
                slots,
            } => self.point(function, Point::Before(stmt), slots),
            Event::End {
                function,
                block,
                slots,
            } => self.point(function, Point::End(block), slots),
            Event::Head {
                function,
                stmt,
                slots,
            } => self.point(function, Point::Head(stmt), slots),
            Event::Given {
                function,
                expr,
                value,
            } => self.value(function, expr, value),
        }
    }
}

/// Tier 1's claims of each function of `module`, checked from `src`, by the function's index.
fn claims_of(src: &str, module: &CheckedModule) -> Vec<Option<Claims>> {
    let mut claims: Vec<Option<Claims>> = (0..module.functions.len()).map(|_| None).collect();
    for (name, states) in mente_sema::tier1_states(src) {
        if let Some(i) = module.index(&name) {
            if let Some(Some(f)) = module.functions.get(i) {
                claims[i] = Some(Claims::of(states, &f.syntax.tree));
            }
        }
    }
    claims
}

/// Runs each call of `calls` traced, watched by `watch`, and keeps its outcome, printed, which
/// is the reference outcome the comparison of the paths judges.
fn watch_calls(watch: &mut Watch<'_>, calls: &[CallArgs], limits: Limits) {
    let ran = mente_interp::with_calls(watch.module, limits, |interp| {
        let mut all = Vec::new();
        for (k, (f, args)) in calls.iter().enumerate() {
            watch.call = k;
            let marked: Vec<bool> = args.iter().map(|(m, _)| *m).collect();
            let values = args.iter().map(|(_, v)| v.clone()).collect();
            let outcome = interp.traced(f, values, &mut |e| watch.event(e));
            all.push(printed(&marked, outcome));
        }
        all
    });
    match ran {
        Ok(all) => watch.out.printed = all,
        Err(stop) => {
            // No call ran: each prints the stop, which the comparison reports as a difference.
            let why = format!("the traced runs did not run: {}", stop);
            watch.out.printed = calls.iter().map(|_| Err(why.clone())).collect();
            watch.out.problems.push((0, why));
        }
    }
}

/// Runs each call of `calls` on the reference interpreter within `limits`, traced, and holds
/// every point it reaches against what Tier 1 claimed there, for the module `module` checked
/// from `src`. A call that stops, by a trap or past the limits, is checked up to where it stops.
pub fn check(src: &str, module: &CheckedModule, calls: &[CallArgs], limits: Limits) -> Outcome {
    let claims = claims_of(src, module);
    let mut watch = Watch::new(src, module, &claims);
    watch_calls(&mut watch, calls, limits);
    watch.out
}

/// Each call's outcome, printed, from the same traced runs as [`check`] makes, watched against no
/// claims: what tracing alone does to a run, for the test that it does nothing.
pub fn traced_outcomes(module: &CheckedModule, calls: &[CallArgs], limits: Limits) -> Vec<Printed> {
    let claims: Vec<Option<Claims>> = (0..module.functions.len()).map(|_| None).collect();
    let mut watch = Watch::new("", module, &claims);
    watch_calls(&mut watch, calls, limits);
    watch.out.printed
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_sema::tier1::states::{Facts, Point, Value as Claim};
    use mente_syntax::tree::StmtId;

    const LIMITS: Limits = Limits {
        max_steps: 1_000_000,
        max_array: 1 << 24,
    };

    fn module(src: &str) -> CheckedModule {
        match mente_sema::check_program(src) {
            Ok(m) => m,
            Err(d) => panic!("{:?}", d),
        }
    }

    fn call(f: &str, args: &[i128]) -> CallArgs {
        (
            f.to_string(),
            args.iter()
                .map(|a| (false, Value::Int(Int::from_i128(*a))))
                .collect(),
        )
    }

    /// Runs `calls` traced against `claims`, which a test makes by hand, in place of Tier 1's.
    fn against(src: &str, claims: Vec<Option<Claims>>, calls: &[CallArgs]) -> Outcome {
        let m = module(src);
        let mut watch = Watch::new(src, &m, &claims);
        watch_calls(&mut watch, calls, LIMITS);
        watch.out
    }

    /// Tier 1's own claims of `src`, each changed by `change`.
    fn claims_changed(src: &str, change: impl Fn(&mut States)) -> Vec<Option<Claims>> {
        let m = module(src);
        let mut claims: Vec<Option<Claims>> = (0..m.functions.len()).map(|_| None).collect();
        for (name, mut states) in mente_sema::tier1_states(src) {
            change(&mut states);
            let i = m.index(&name).unwrap();
            let tree = &m.functions[i].as_ref().unwrap().syntax.tree;
            claims[i] = Some(Claims::of(states, tree));
        }
        claims
    }

    const SUM: &str = "module m\npub fn sum(n: u8) -> u16 {\n  var s: u16 = 0\n  var i: u8 = 0\n  while i < n {\n    s = s + 1\n    i = i + 1\n  }\n  s\n}\n";

    #[test]
    fn test_tier1_states_contain_every_value_the_interpreter_reaches() {
        let srcs = [
            SUM,
            "module m\npub fn f(x: u8, y: u8) -> u8 {\n  guard x < y else { return 0 }\n  val d: u8 = y - x\n  var r: u8 = x\n  r %= d\n  r\n}\n",
            "module m\nstruct P {\n  a: u8\n  b: i8\n}\npub fn g(x: u8) -> i16 {\n  val p = P(a: x, b: -3)\n  val k: [3]u8 = [x, 1, 2]\n  if p.a > 9 { p.a + k[2] } else { p.b - 1 }\n}\n",
            // Routed clauses that hold blocks, which Tier 1 walks nowhere though a run evaluates
            // them, and a match with an arm Tier 1 holds unreachable.
            "module m\n@checked\npub fn h(x: u8) -> u8\n  ensures (if result > 3 { result > 3 } else { result <= 3 })\n{\n  val y: u8 = x / 2\n  y\n}\n@prove\npub fn k(x: u8) -> u8\n  ensures (match result {\n    0 => true\n    _ => { result > 0 }\n  })\n{\n  x\n}\npub fn n(x: u8) -> u8 {\n  guard x > 0 else { return 1 }\n  match x {\n    0 => 7\n    _ => x\n  }\n}\n",
        ];
        let calls: [&[CallArgs]; 4] = [
            &[call("sum", &[0]), call("sum", &[5]), call("sum", &[255])],
            &[
                call("f", &[3, 10]),
                call("f", &[10, 3]),
                call("f", &[0, 255]),
                call("f", &[254, 255]),
            ],
            &[call("g", &[0]), call("g", &[10]), call("g", &[255])],
            &[
                call("h", &[0]),
                call("h", &[8]),
                call("h", &[255]),
                call("k", &[0]),
                call("k", &[200]),
                call("n", &[0]),
                call("n", &[1]),
                call("n", &[9]),
            ],
        ];
        for (src, calls) in srcs.iter().zip(calls) {
            let m = module(src);
            let out = check(src, &m, calls, LIMITS);
            assert!(out.problems.is_empty(), "{}: {:?}", src, out.problems);
            assert!(out.points > 0 && out.values > 0, "{}: {:?}", src, out);
        }
    }

    #[test]
    fn test_a_value_outside_a_recorded_bound_is_a_problem() {
        // `s` holds at most 255 at the loop's head; the claim says 4.
        let claims = claims_changed(SUM, |st| {
            let s = st.tracked.iter().find(|t| t.binding == 1).unwrap().var;
            for (p, f) in &mut st.points {
                if matches!(p, Point::Head(_)) {
                    for b in &mut f.bounds {
                        if b.0 == s {
                            b.2 = Some(Exact::from_i128(4));
                        }
                    }
                }
            }
        });
        let out = against(SUM, claims.clone(), &[call("sum", &[4])]);
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        let out = against(SUM, claims, &[call("sum", &[6])]);
        assert_eq!(out.problems.len(), 1, "{:?}", out.problems);
        let (_, text) = &out.problems[0];
        assert!(
            text.contains("at the head of a round of the loop at 5:3")
                && text.contains("`s` is 5, outside [0, 4]"),
            "{}",
            text
        );
    }

    #[test]
    fn test_a_broken_relation_is_a_problem() {
        // At the head, `s - i <= 0` holds; a claim of `s - i <= -1` does not.
        let claims = claims_changed(SUM, |st| {
            let var = |b: usize| st.tracked.iter().find(|t| t.binding == b).unwrap().var;
            let (s, i) = (var(1), var(2));
            for (p, f) in &mut st.points {
                if matches!(p, Point::Head(_)) {
                    f.relations.push((
                        Signed { var: s, neg: false },
                        Signed { var: i, neg: true },
                        -1,
                    ));
                }
            }
        });
        let out = against(SUM, claims, &[call("sum", &[2])]);
        assert_eq!(out.problems.len(), 1, "{:?}", out.problems);
        assert!(
            out.problems[0]
                .1
                .contains("`s` is 0 and `i` is 0, against s + -i <= -1"),
            "{}",
            out.problems[0].1
        );
    }

    #[test]
    fn test_an_empty_or_missing_state_where_the_interpreter_reaches_is_a_problem() {
        let emptied = claims_changed(SUM, |st| {
            for (p, f) in &mut st.points {
                if matches!(p, Point::Before(StmtId(_))) && !f.empty {
                    f.empty = true;
                    f.bounds.clear();
                    f.relations.clear();
                }
            }
        });
        let out = against(SUM, emptied, &[call("sum", &[1])]);
        assert!(
            out.problems
                .iter()
                .any(|(_, t)| t.contains("Tier 1's state there is empty")),
            "{:?}",
            out.problems
        );
        let missing = claims_changed(SUM, |st| {
            st.points.retain(|(p, _)| !matches!(p, Point::End(_)));
        });
        let out = against(SUM, missing, &[call("sum", &[1])]);
        assert!(
            out.problems
                .iter()
                .any(|(_, t)| t.contains("at the end of the block") && t.contains("no state")),
            "{:?}",
            out.problems
        );
        // A state recorded twice holds either: one empty record beside a live one is no problem.
        let doubled = claims_changed(SUM, |st| {
            let extra: Vec<_> = st
                .points
                .iter()
                .map(|(p, _)| {
                    (
                        *p,
                        Facts {
                            empty: true,
                            ..Facts::default()
                        },
                    )
                })
                .collect();
            st.points.extend(extra);
        });
        let out = against(SUM, doubled, &[call("sum", &[3])]);
        assert!(out.problems.is_empty(), "{:?}", out.problems);
    }

    #[test]
    fn test_an_expression_value_outside_its_recorded_interval_is_a_problem() {
        // The claim that `s + 1` is at most 2, and that `i + 1` was evaluated in an empty state.
        let src = SUM;
        let m = module(src);
        let tree = &m.functions[0].as_ref().unwrap().syntax.tree;
        let find = |text: &str| {
            (0..tree.exprs.len() as u32)
                .map(ExprId)
                .find(|&e| {
                    let sp = tree.expr(e).span;
                    &src[sp.start..sp.end] == text
                })
                .unwrap()
        };
        let (s1, i1) = (find("s + 1"), find("i + 1"));
        let claims = claims_changed(src, |st| {
            for v in &mut st.values {
                if v.expr == s1 {
                    v.hi = Some(Exact::from_i128(2));
                }
                if v.expr == i1 {
                    *v = Claim {
                        expr: i1,
                        empty: true,
                        lo: None,
                        hi: None,
                    };
                }
            }
        });
        let out = against(src, claims, &[call("sum", &[3])]);
        let texts: Vec<&str> = out.problems.iter().map(|(_, t)| t.as_str()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("the value of `s + 1` at 6:9") && t.contains("gives 3")),
            "{:?}",
            texts
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("`i + 1`") && t.contains("in an empty state")),
            "{:?}",
            texts
        );
        // An expression with no record is no claim.
        let none = claims_changed(src, |st| st.values.clear());
        assert!(against(src, none, &[call("sum", &[3])]).problems.is_empty());
    }
}

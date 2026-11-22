//! The octagon domain over integers: a difference-bound matrix over `+x` and `-x` for each
//! variable (`spec/ghost-contracts-and-totality.md`, item 4), kept tightly closed. Ported from
//! the M0b spike (`spikes/tier1-latency/src/octagon.rs`).
//!
//! Entry `m[i][j]` bounds `V_j - V_i`, where `V_2k = +x_k` and `V_2k+1 = -x_k`. Adding one
//! constraint costs O(n^2): a shortest-path relaxation for the new edge and its coherent twin,
//! then tightening and strengthening (Mine; Bagnara, Hill and Zaffanella for integers).
//!
//! The matrix keeps constants up to [`BIG`]; the 128-bit types' ranges reach past it. A bound of
//! one variable beyond it, from such a type, a refinement or a `guard`, is kept exactly beside
//! the matrix, as an [`Exact`], so that a value of `u128[0..u128::MAX]` is known to lie below
//! its bound though the matrix cannot say so.
//!
//! Past [`RELATIONAL_LIMIT`] variables it is a box instead: each variable's bounds alone, which
//! cost linear time where the matrix costs quadratic, so that Tier 1's cost stays bounded however
//! large a function grows. A relation between two variables then narrows each one's bounds by
//! the other's, once, and is dropped, which is sound and only weaker.
//!
//! Each variable may carry a stamp: the evaluation whose value it holds, which Tier 1 gives a
//! site variable, the value of a remainder, an `if`, a `match`, an index, a field or a call, each
//! time it evaluates that expression. A term of a value names the evaluation it reads. Every
//! method that takes terms reads them, and stops the compile, in every build, where a term names
//! one evaluation and the state holds another, two joined, or one the term does not name: no
//! value is read from an evaluation the state does not hold (`spec/ghost-contracts-and-
//! totality.md`, item 2, `no-site-value-from-another-evaluation`). The bounds themselves never
//! depend on the stamps, and two octagons that differ in stamps alone are equal.

use std::sync::Arc;

/// The most variables an octagon relates; past them it is a box. The largest function of the
/// repository's corpus has 53 (Norman's decision, `tier1-variable-cap`).
pub const RELATIONAL_LIMIT: usize = 128;

/// The stamp of a variable whose value came from two evaluations, joined.
pub const MIXED: u32 = u32::MAX;

/// No bound. Sums saturate here; a bound at or beyond it is no bound, so the 128-bit types,
/// whose ranges reach past it, are unbounded in the domain.
pub const INF: i128 = i128::MAX / 4;

/// The largest magnitude a constraint's constant keeps: beyond it a constraint is weakened,
/// which is sound, so that doubling a constant never overflows.
pub const BIG: i128 = INF / 2;

/// A sum of two upper bounds, kept within `[-inf, inf]`, the matrix's limit: at or beyond `inf`
/// it is no bound, and below `-inf` it is raised to `-inf`, a weaker bound, which is sound and
/// keeps every later sum of two bounds inside `i128`. Every entry of the matrix is an upper
/// bound, so `inf` is always "none" and `-inf` always a value.
fn add(inf: i128, a: i128, b: i128) -> i128 {
    if a >= inf || b >= inf {
        inf
    } else {
        (a + b).clamp(-inf, inf)
    }
}

fn floor_half(v: i128) -> i128 {
    v.div_euclid(2)
}

/// An integer of the range the 128-bit types span together, `-(2^128 - 1)` to `2^128 - 1`,
/// kept exactly: a bound beyond the matrix's reach. Zero is never negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Exact {
    negative: bool,
    magnitude: u128,
}

impl Exact {
    pub fn new(negative: bool, magnitude: u128) -> Exact {
        Exact {
            negative: negative && magnitude != 0,
            magnitude,
        }
    }

    pub fn from_i128(v: i128) -> Exact {
        Exact::new(v.is_negative(), v.unsigned_abs())
    }

    /// The value as an `i128`, when it is one.
    pub fn to_i128(self) -> Option<i128> {
        if self.negative {
            0i128.checked_sub_unsigned(self.magnitude)
        } else {
            i128::try_from(self.magnitude).ok()
        }
    }

    pub fn negated(self) -> Exact {
        Exact::new(!self.negative, self.magnitude)
    }

    /// `self + d`, or `None` past the range.
    pub fn plus(self, d: i128) -> Option<Exact> {
        let d = Exact::from_i128(d);
        if self.negative == d.negative {
            let m = self.magnitude.checked_add(d.magnitude)?;
            Some(Exact::new(self.negative, m))
        } else if self.magnitude >= d.magnitude {
            Some(Exact::new(self.negative, self.magnitude - d.magnitude))
        } else {
            Some(Exact::new(d.negative, d.magnitude - self.magnitude))
        }
    }

    /// Whether the matrix cannot keep it as a constant.
    pub fn is_far(self) -> bool {
        self.magnitude >= BIG.unsigned_abs()
    }
}

impl Ord for Exact {
    fn cmp(&self, other: &Exact) -> std::cmp::Ordering {
        match (self.negative, other.negative) {
            (false, false) => self.magnitude.cmp(&other.magnitude),
            (true, true) => other.magnitude.cmp(&self.magnitude),
            (false, true) => std::cmp::Ordering::Greater,
            (true, false) => std::cmp::Ordering::Less,
        }
    }
}

impl PartialOrd for Exact {
    fn partial_cmp(&self, other: &Exact) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Exact {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.negative {
            write!(f, "-{}", self.magnitude)
        } else {
            write!(f, "{}", self.magnitude)
        }
    }
}

/// One term of a linear expression: `+x` or `-x`, and the evaluation of `x` it reads, 0 for
/// a variable no evaluation stamps ([`Octagon::stamp`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Term {
    pub var: usize,
    pub neg: bool,
    pub at: u32,
}

impl Term {
    pub fn pos(var: usize) -> Term {
        Term {
            var,
            neg: false,
            at: 0,
        }
    }

    /// `+x`, read as evaluation `at` of it.
    pub fn of(var: usize, at: u32) -> Term {
        Term {
            var,
            neg: false,
            at,
        }
    }

    pub fn negated(self) -> Term {
        Term {
            var: self.var,
            neg: !self.neg,
            at: self.at,
        }
    }

    fn node(self) -> usize {
        2 * self.var + usize::from(self.neg)
    }
}

#[derive(Clone, Debug)]
pub struct Octagon {
    n: usize,
    /// The matrix, `2n` by `2n`; or, for a box, each node's unary entry alone: `m[a]` bounds
    /// `V_a - V_(a ^ 1)`, twice the node's term.
    m: Vec<i128>,
    /// Whether it is a box, past [`RELATIONAL_LIMIT`] variables.
    boxed: bool,
    empty: bool,
    /// Each variable's lower and upper bounds beyond the matrix's reach, kept exactly; none
    /// until one is added, so that an octagon stays small where no 128-bit bound arises.
    far: Option<Box<[FarBounds]>>,
    /// The matrix's limit: an entry at it is no bound. [`INF`] for every octagon Tier 1
    /// makes; a test makes one with a small limit, and takes every constant around it.
    inf: i128,
    /// Each variable's stamp: the evaluation whose value it holds, 0 for none, [`MIXED`] for two
    /// joined; none until one is given.
    stamps: Option<Box<[u32]>>,
    /// What each variable is, for the internal error a stale read stops the compile with.
    names: Option<Arc<[String]>>,
}

/// A variable's lower and upper bounds beyond the matrix's reach.
type FarBounds = (Option<Exact>, Option<Exact>);

/// Two octagons are equal when they hold the same bounds: a table of far bounds made but empty
/// is no table, and the stamps and names are no bounds.
impl PartialEq for Octagon {
    fn eq(&self, other: &Octagon) -> bool {
        self.n == other.n
            && self.inf == other.inf
            && self.boxed == other.boxed
            && self.empty == other.empty
            && self.m == other.m
            && (0..self.n).all(|v| self.far_of(v) == other.far_of(v))
    }
}

impl Eq for Octagon {}

impl Octagon {
    /// The octagon of `n` unconstrained variables: a box past [`RELATIONAL_LIMIT`].
    pub fn new(n: usize) -> Octagon {
        Octagon::within(n, INF)
    }

    /// [`Octagon::new`], with a matrix whose limit is `inf`.
    pub fn within(n: usize, inf: i128) -> Octagon {
        let d = 2 * n;
        if n > RELATIONAL_LIMIT {
            return Octagon {
                n,
                m: vec![inf; d],
                boxed: true,
                empty: false,
                far: None,
                inf,
                stamps: None,
                names: None,
            };
        }
        let mut m = vec![inf; d * d];
        for k in 0..d {
            m[k * d + k] = 0;
        }
        Octagon {
            n,
            m,
            boxed: false,
            empty: false,
            far: None,
            inf,
            stamps: None,
            names: None,
        }
    }

    /// The bounds of `var` kept beyond the matrix's reach.
    fn far_of(&self, var: usize) -> FarBounds {
        self.far.as_ref().map_or((None, None), |f| f[var])
    }

    /// The table of bounds beyond the matrix's reach, made on first use.
    fn far_mut(&mut self) -> &mut [FarBounds] {
        let n = self.n;
        self.far
            .get_or_insert_with(|| vec![(None, None); n].into_boxed_slice())
    }

    /// Adds `lower <= var <= upper`, either side optional: a bound the matrix can keep goes
    /// there, and one beyond its reach beside it, exactly.
    pub fn add_exact(&mut self, var: usize, lower: Option<Exact>, upper: Option<Exact>) {
        if self.empty {
            return;
        }
        if let Some(l) = lower {
            match l.to_i128().filter(|_| !l.is_far()) {
                Some(c) => self.add_lin_of(&[Term::pos(var).negated()], -c),
                None => {
                    let kept = &mut self.far_mut()[var].0;
                    *kept = Some(kept.map_or(l, |k| k.max(l)));
                }
            }
        }
        if let Some(u) = upper {
            match u.to_i128().filter(|_| !u.is_far()) {
                Some(c) => self.add_lin_of(&[Term::pos(var)], c),
                None => {
                    let kept = &mut self.far_mut()[var].1;
                    *kept = Some(kept.map_or(u, |k| k.min(u)));
                }
            }
        }
        if let (Some(l), Some(u)) = self.exact_bounds(var) {
            if l > u {
                self.empty = true;
            }
        }
    }

    /// The bounds of `var` kept beyond the matrix's reach alone.
    pub fn far_bounds(&self, var: usize) -> (Option<Exact>, Option<Exact>) {
        self.far_of(var)
    }

    /// [`Octagon::far_bounds`] of a term's variable, the term read ([`Octagon::read`]).
    pub fn far_bounds_of(&self, t: Term) -> (Option<Exact>, Option<Exact>) {
        self.read(&[t]);
        self.far_of(t.var)
    }

    /// [`Octagon::add_exact`] to a term's variable, the term read as [`Octagon::add_lin`]
    /// reads it.
    pub fn add_exact_of(&mut self, t: Term, lower: Option<Exact>, upper: Option<Exact>) {
        self.narrowed_by(&[t]);
        self.add_exact(t.var, lower, upper);
    }

    /// Reads `terms` where they narrow the state ([`Octagon::read`]): each variable then holds
    /// the evaluation its term names, which the read leaves as it was or, where the state held
    /// none, gives it, so that a later term of another evaluation is refused there.
    fn narrowed_by(&mut self, terms: &[Term]) {
        self.read(terms);
        for t in terms {
            self.stamp(t.var, t.at);
        }
    }

    /// The bounds of `var`, exactly, from the matrix and beyond it; `None` for none.
    pub fn exact_bounds(&self, var: usize) -> (Option<Exact>, Option<Exact>) {
        let (lo, hi) = self.bounds(var);
        let (far_lo, far_hi) = self.far_of(var);
        let lo = (lo > -self.inf).then(|| Exact::from_i128(lo));
        let hi = (hi < self.inf).then(|| Exact::from_i128(hi));
        let tighter = |a: Option<Exact>, b: Option<Exact>, upper: bool| match (a, b) {
            (Some(a), Some(b)) => Some(if upper { a.min(b) } else { a.max(b) }),
            (a, b) => a.or(b),
        };
        (tighter(lo, far_lo, false), tighter(hi, far_hi, true))
    }

    /// The bounds kept beyond the matrix's reach: each variable with any, and its lower and
    /// upper ones. What the SMT cross-check is given of them (`tier1::smt`).
    pub fn far_constraints(&self) -> Vec<(usize, Option<Exact>, Option<Exact>)> {
        self.far.as_ref().map_or(Vec::new(), |f| {
            f.iter()
                .enumerate()
                .filter(|(_, (l, u))| l.is_some() || u.is_some())
                .map(|(v, (l, u))| (v, *l, *u))
                .collect()
        })
    }

    /// Whether it relates its variables, rather than keeping their bounds alone.
    pub fn is_relational(&self) -> bool {
        !self.boxed
    }

    /// A node's unary entry: the bound of twice its term.
    fn unary(&self, a: usize) -> i128 {
        if self.boxed {
            self.m[a]
        } else {
            self.at(a ^ 1, a)
        }
    }

    /// A box's new bound `v` on twice node `a`'s term, tightened to an even number; a
    /// variable whose two bounds cross empties the box.
    fn bound_box(&mut self, a: usize, v: i128) {
        if v >= self.inf || v >= self.m[a] {
            return;
        }
        self.m[a] = (2 * floor_half(v)).max(-self.inf);
        let k = a & !1;
        if add(self.inf, self.m[k], self.m[k + 1]) < 0 {
            self.empty = true;
        }
    }

    /// Adds `terms <= c` to a box: a bound on one term; and for two, each one's bound by the
    /// other's least value, once.
    fn add_box(&mut self, terms: &[Term], c: i128) {
        match terms {
            [] => {
                if c < 0 {
                    self.empty = true;
                }
            }
            [t] => self.bound_box(t.node(), 2 * c),
            [s, t] if s.var == t.var => {
                if s.neg == t.neg {
                    self.bound_box(s.node(), c);
                } else if c < 0 {
                    self.empty = true;
                }
            }
            [s, t] => {
                // `s + t <= c` and `-t <= u` give `s <= c + u`; with no `u`, the sum saturates
                // at `INF`, no bound, which `bound_box` passes over.
                for (a, b) in [(*s, *t), (*t, *s)] {
                    let v = add(self.inf, c, self.upper_of(&[b.negated()]));
                    self.bound_box(a.node(), 2 * v);
                }
            }
            _ => {}
        }
    }

    pub fn vars(&self) -> usize {
        self.n
    }

    fn d(&self) -> usize {
        2 * self.n
    }

    fn at(&self, i: usize, j: usize) -> i128 {
        self.m[i * self.d() + j]
    }

    fn put(&mut self, i: usize, j: usize, v: i128) {
        let d = self.d();
        self.m[i * d + j] = v;
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    pub fn set_empty(&mut self) {
        self.empty = true;
    }

    /// Adds `V_j - V_i <= c` and its coherent twin, then restores tight closure.
    fn add_edge(&mut self, i: usize, j: usize, c: i128) {
        if self.empty {
            return;
        }
        self.relax(i, j, c);
        self.relax(j ^ 1, i ^ 1, c);
        self.tighten_and_strengthen();
    }

    fn relax(&mut self, i: usize, j: usize, c: i128) {
        let d = self.d();
        let into_i: Vec<i128> = (0..d).map(|k| self.at(k, i)).collect();
        let from_j: Vec<i128> = (0..d).map(|l| self.at(j, l)).collect();
        for (k, ki) in into_i.iter().enumerate() {
            let a = add(self.inf, *ki, c);
            for (l, jl) in from_j.iter().enumerate() {
                let v = add(self.inf, a, *jl).min(self.at(k, l));
                self.put(k, l, v);
            }
        }
    }

    fn tighten_and_strengthen(&mut self) {
        let d = self.d();
        for k in 0..d {
            let v = self.at(k, k ^ 1);
            if v < self.inf {
                self.put(k, k ^ 1, (2 * floor_half(v)).max(-self.inf));
            }
        }
        for i in 0..d {
            let ci = self.at(i, i ^ 1);
            if ci >= self.inf {
                continue;
            }
            for j in 0..d {
                let cj = self.at(j ^ 1, j);
                if cj >= self.inf {
                    continue;
                }
                // Both are below `INF` and at or above `-INF`, so their sum fits; it is not
                // saturated, since half of it is the bound.
                let s = floor_half(ci + cj).min(self.at(i, j));
                self.put(i, j, s);
            }
        }
        if (0..d).any(|k| self.at(k, k) < 0) {
            self.empty = true;
        }
    }

    /// Adds `terms <= c`, for at most two terms; a longer sum is ignored, which is sound. A
    /// constant beyond [`BIG`] is weakened to no bound, or to `-BIG`. It reads its terms
    /// ([`Octagon::read`]), and a term that narrows a variable no evaluation holds stamps it
    /// with its own, so that a later term of another evaluation is refused there.
    pub fn add_lin(&mut self, terms: &[Term], c: i128) {
        self.narrowed_by(terms);
        self.add_lin_of(terms, c);
    }

    /// [`Octagon::add_lin`], its terms read.
    fn add_lin_of(&mut self, terms: &[Term], c: i128) {
        if c >= (self.inf / 2) || self.empty {
            return;
        }
        let c = c.max(-(self.inf / 2));
        if self.boxed {
            self.add_box(terms, c);
        } else {
            self.add_matrix(terms, c);
        }
        self.check_far();
    }

    /// Empties the octagon where a variable's bounds in the matrix and beyond it leave no value.
    fn check_far(&mut self) {
        let Some(far) = &self.far else {
            return;
        };
        let kept: Vec<usize> = far
            .iter()
            .enumerate()
            .filter(|(_, (l, u))| l.is_some() || u.is_some())
            .map(|(v, _)| v)
            .collect();
        for v in kept {
            if let (Some(l), Some(u)) = self.exact_bounds(v) {
                if l > u {
                    self.empty = true;
                    return;
                }
            }
        }
    }

    fn add_matrix(&mut self, terms: &[Term], c: i128) {
        match terms {
            [] => {
                if c < 0 {
                    self.empty = true;
                }
            }
            [t] => {
                let a = t.node();
                self.add_edge(a ^ 1, a, 2 * c);
            }
            // Two terms of one variable work too: `x + x <= c` bounds `2x`, and `x - x <= c`
            // puts `c` on the diagonal, where a negative one empties the octagon.
            [s, t] => {
                let (a, b) = (s.node(), t.node());
                self.add_edge(b ^ 1, a, c);
            }
            _ => {}
        }
    }

    /// The least upper bound of `terms` (at most two), or `INF`, its terms read
    /// ([`Octagon::read`]).
    pub fn upper(&self, terms: &[Term]) -> i128 {
        self.read(terms);
        self.upper_of(terms)
    }

    /// [`Octagon::upper`], its terms read, or the octagon's own.
    fn upper_of(&self, terms: &[Term]) -> i128 {
        match terms {
            [] => 0,
            [t] => {
                let v = self.unary(t.node());
                if v >= self.inf {
                    self.inf
                } else {
                    floor_half(v)
                }
            }
            // A box relates nothing: two terms are at most the sum of each one's bound, or, of
            // one variable, twice it or nothing.
            [s, t] if self.boxed => match (s.var == t.var, s.neg == t.neg) {
                (true, true) => self.unary(s.node()),
                (true, false) => 0,
                (false, _) => add(self.inf, self.upper_of(&[*s]), self.upper_of(&[*t])),
            },
            [s, t] => self.at(t.node() ^ 1, s.node()),
            _ => self.inf,
        }
    }

    /// Whether `terms <= c` holds in every point of the octagon, its terms read.
    pub fn implies(&self, terms: &[Term], c: i128) -> bool {
        self.read(terms);
        self.empty || self.upper_of(terms) <= c
    }

    /// The bounds of one variable, with `-INF` and `INF` for none.
    pub fn bounds(&self, var: usize) -> (i128, i128) {
        let hi = self.upper_of(&[Term::pos(var)]);
        let lo = self.upper_of(&[Term::pos(var).negated()]);
        crate::bounds::held(self.inf, (if lo >= self.inf { -self.inf } else { -lo }, hi))
    }

    /// Forgets everything known about `var`, the evaluation it held included.
    pub fn forget(&mut self, var: usize) {
        if let Some(f) = &mut self.far {
            f[var] = (None, None);
        }
        if let Some(s) = &mut self.stamps {
            s[var] = 0;
        }
        if self.boxed {
            self.m[2 * var] = self.inf;
            self.m[2 * var + 1] = self.inf;
            return;
        }
        let d = self.d();
        for node in [2 * var, 2 * var + 1] {
            for k in 0..d {
                if k != node {
                    self.put(node, k, self.inf);
                    self.put(k, node, self.inf);
                }
            }
        }
    }

    /// The smallest octagon holding both: the entry-wise maximum of two closed octagons.
    pub fn join(&self, other: &Octagon) -> Octagon {
        if self.empty {
            return other.clone();
        }
        if other.empty {
            return self.clone();
        }
        let m = self
            .m
            .iter()
            .zip(&other.m)
            .map(|(a, b)| *a.max(b))
            .collect();
        // A bound beyond the matrix holds after the join where it holds on both sides, the
        // weaker of the two.
        let far = match (&self.far, &other.far) {
            (Some(a), Some(b)) => Some(
                a.iter()
                    .zip(b.iter())
                    .map(|(a, b)| {
                        let lower = a.0.zip(b.0).map(|(x, y)| x.min(y));
                        let upper = a.1.zip(b.1).map(|(x, y)| x.max(y));
                        (lower, upper)
                    })
                    .collect(),
            ),
            _ => None,
        };
        Octagon {
            n: self.n,
            m,
            boxed: self.boxed,
            empty: false,
            far,
            inf: self.inf,
            stamps: self.joined_stamps(other),
            names: self.names.clone().or_else(|| other.names.clone()),
        }
    }

    /// Each variable's stamp after a join: one evaluation where both sides hold it, or one holds
    /// it and the other none, and [`MIXED`] where they hold two.
    fn joined_stamps(&self, other: &Octagon) -> Option<Box<[u32]>> {
        if self.stamps.is_none() && other.stamps.is_none() {
            return None;
        }
        Some(
            (0..self.n)
                .map(|v| match (self.stamp_of(v), other.stamp_of(v)) {
                    (a, b) if a == b => a,
                    (0, b) => b,
                    (a, 0) => a,
                    (_, _) => MIXED,
                })
                .collect(),
        )
    }

    /// The evaluation whose value `var` holds: 0 for none, [`MIXED`] for two joined.
    pub fn stamp_of(&self, var: usize) -> u32 {
        self.stamps.as_ref().map_or(0, |s| s[var])
    }

    /// Stamps `var` with evaluation `at`, once that evaluation has given it its value. A write
    /// forgets its variable first, so the state holds no evaluation of it, or already this one;
    /// a variable that holds another, or two joined, stops the compile, in every build, with an
    /// internal error that names it: its bounds would be the earlier evaluation's under the new
    /// stamp, the stale fact the stamps exist to refuse.
    pub fn stamp(&mut self, var: usize, at: u32) {
        let held = self.stamp_of(var);
        if held != 0 && held != at {
            panic!(
                "internal error: {} is given a new evaluation where the state holds an earlier one, which its write did not forget (ghost-contracts-and-totality.md, item 2, no-site-value-from-another-evaluation)",
                self.name_of(var)
            );
        }
        self.set_stamp(var, at);
    }

    /// Sets `var`'s stamp to `at`, whatever it held: a replay's rebasing alone, and
    /// [`Octagon::stamp`] once it has checked.
    fn set_stamp(&mut self, var: usize, at: u32) {
        if at == 0 && self.stamps.is_none() {
            return;
        }
        let n = self.n;
        self.stamps
            .get_or_insert_with(|| vec![0; n].into_boxed_slice())[var] = at;
    }

    /// What `var` is, as the internal errors of the stamps name it.
    fn name_of(&self, var: usize) -> String {
        self.names
            .as_ref()
            .and_then(|n| n.get(var).cloned())
            .unwrap_or_else(|| format!("variable {}", var))
    }

    /// For a state computed from entry `from` and replayed at entry `to`, which hold the same
    /// bounds: each variable that holds `from`'s evaluation, left as it was entered, holds
    /// `to`'s. A variable that holds two joined keeps them.
    pub fn restamp(&mut self, from: &Octagon, to: &Octagon) {
        for v in 0..self.n {
            let held = self.stamp_of(v);
            if held != MIXED && held == from.stamp_of(v) {
                self.set_stamp(v, to.stamp_of(v));
            }
        }
    }

    /// Names each variable for the internal error a stale read stops the compile with.
    pub fn name(&mut self, names: Arc<[String]>) {
        self.names = Some(names);
    }

    /// Reads `terms`: each names the evaluation the state holds, or the state holds none of its
    /// variable. Anything else stops the compile, in every build, with an internal error that
    /// names the variable: a value read from an evaluation the state does not hold would be
    /// decided on another value.
    fn read(&self, terms: &[Term]) {
        for t in terms {
            let held = self.stamp_of(t.var);
            if held == 0 || held == t.at {
                continue;
            }
            let what = match (t.at, held) {
                (0, _) => "is read with no evaluation named, where the state holds one",
                (_, MIXED) => "is read as one evaluation, where the state holds two joined",
                (_, _) => "is read as one evaluation, where the state holds another",
            };
            panic!(
                "internal error: {} {} (ghost-contracts-and-totality.md, item 2, no-site-value-from-another-evaluation)",
                self.name_of(t.var),
                what
            );
        }
    }

    /// The constraints the matrix holds, each as `terms <= c`: every bound off the diagonal, a
    /// coherent pair once, and each variable's diagonal entry below zero, which makes the octagon
    /// empty, as `[] <= c`; a constant constraint appears only there. What the SMT cross-check is
    /// given of a state (`tier1::smt`).
    pub fn constraints(&self) -> Vec<(Vec<Term>, i128)> {
        let d = self.d();
        let term = |node: usize| Term {
            var: node / 2,
            neg: node % 2 == 1,
            at: 0,
        };
        let mut out = Vec::new();
        if self.boxed {
            // Each node's bound on twice its term, and an empty box as a false constant.
            if self.empty {
                out.push((Vec::new(), -1));
            }
            for a in 0..d {
                if self.m[a] < self.inf {
                    out.push((vec![term(a), term(a)], self.m[a]));
                }
            }
            return out;
        }
        for i in 0..d {
            // An entry `(i, j)` and its coherent twin `(j ^ 1, i ^ 1)` hold one bound, and the
            // twin is the one with `j <= i ^ 1`: each pair is listed once, and a unary bound or a
            // diagonal entry, which pairs with itself or its variable's other node, once.
            for j in (i ^ 1)..d {
                let c = self.at(i, j);
                if i == j {
                    if c < 0 {
                        out.push((Vec::new(), c));
                    }
                } else if c < self.inf {
                    // `V_j - V_i <= c`, and `-V_i` is the node `i ^ 1`.
                    out.push((vec![term(j), term(i ^ 1)], c));
                }
            }
        }
        out
    }

    /// The bounds of `var`, exactly, with those each relation of the matrix gives it from
    /// another variable's: the matrix keeps no bound beyond its reach, so it cannot carry one
    /// along a relation itself. `None` for none.
    fn related_bounds(&self, var: usize) -> (Option<Exact>, Option<Exact>) {
        let (mut lo, mut hi) = self.exact_bounds(var);
        for w in 0..self.n {
            let (wlo, whi) = self.exact_bounds(w);
            for tv in [Term::pos(var), Term::pos(var).negated()] {
                for tw in [Term::pos(w), Term::pos(w).negated()] {
                    // `tv + tw <= c` bounds `tv` by `c` less the least value of `tw`.
                    let c = self.upper_of(&[tv, tw]);
                    let least = if tw.neg { whi.map(Exact::negated) } else { wlo };
                    let Some(b) = least
                        .filter(|_| c < self.inf)
                        .and_then(|l| l.negated().plus(c))
                    else {
                        continue;
                    };
                    if tv.neg {
                        lo = Some(lo.map_or(b.negated(), |l| l.max(b.negated())));
                    } else {
                        hi = Some(hi.map_or(b, |h| h.min(b)));
                    }
                }
            }
        }
        (lo, hi)
    }

    /// A point of the octagon: each of `vars` in turn takes the value within its bounds, in the
    /// matrix, beyond it and through its relations, that is closest to zero, which is then
    /// fixed, so the result satisfies every constraint.
    pub fn pick(&self, vars: &[usize]) -> Option<Vec<Exact>> {
        if self.empty {
            return None;
        }
        let zero = Exact::from_i128(0);
        let mut s = self.clone();
        let mut out = Vec::new();
        for &v in vars {
            let (lo, hi) = s.related_bounds(v);
            let value = lo.map_or(zero, |l| l.max(zero));
            let value = hi.map_or(value, |h| h.min(value));
            s.add_exact(v, Some(value), Some(value));
            if s.empty {
                return None;
            }
            out.push(value);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x(v: usize) -> Term {
        Term::pos(v)
    }
    fn nx(v: usize) -> Term {
        Term {
            var: v,
            neg: true,
            at: 0,
        }
    }

    /// An exact integer orders, shifts and converts across the whole range of the 128-bit
    /// types, and knows which values the matrix cannot keep.
    #[test]
    fn test_exact_integers_span_the_128_bit_types() {
        let max = Exact::new(false, u128::MAX);
        let min = Exact::new(true, 1u128 << 127);
        assert!(min < Exact::from_i128(-1) && Exact::from_i128(-1) < Exact::from_i128(0));
        assert!(Exact::from_i128(0) < max);
        assert_eq!(Exact::new(true, 0), Exact::from_i128(0));
        assert_eq!(max.plus(1), None);
        assert_eq!(max.plus(-1), Some(Exact::new(false, u128::MAX - 1)));
        assert_eq!(Exact::from_i128(-3).plus(5), Some(Exact::from_i128(2)));
        assert_eq!(Exact::from_i128(3).plus(-5), Some(Exact::from_i128(-2)));
        assert_eq!(min.to_i128(), Some(i128::MIN));
        assert_eq!(max.to_i128(), None);
        assert_eq!(min.negated().to_i128(), None);
        assert_eq!(max.to_string(), "340282366920938463463374607431768211455");
        assert_eq!(min.to_string(), "-170141183460469231731687303715884105728");
        assert!(max.is_far() && min.is_far());
        assert!(!Exact::from_i128(BIG - 1).is_far() && Exact::from_i128(BIG).is_far());
    }

    /// A bound beyond the matrix's reach is kept exactly beside it: it narrows the variable, a
    /// contradiction with the matrix empties the octagon, a join keeps the weaker of two, a
    /// variable forgotten loses it, and a valuation respects it. A bound within reach goes in
    /// the matrix.
    #[test]
    fn test_bounds_beyond_the_matrix_are_kept_exactly() {
        let max = Exact::new(false, u128::MAX);
        let mut o = Octagon::new(2);
        o.add_exact(0, Some(Exact::from_i128(0)), Some(max));
        assert_eq!(o.bounds(0), (0, INF));
        assert_eq!(o.far_bounds(0), (None, Some(max)));
        assert_eq!(o.exact_bounds(0), (Some(Exact::from_i128(0)), Some(max)));
        assert_eq!(o.far_constraints(), vec![(0, None, Some(max))]);
        // Tighter wins, looser is ignored.
        let below = max.plus(-1).unwrap();
        o.add_exact(0, None, Some(below));
        o.add_exact(0, None, Some(max));
        assert_eq!(o.far_bounds(0), (None, Some(below)));
        // A lower bound past the upper one leaves no value, beyond the matrix or within it.
        let mut empty = o.clone();
        empty.add_exact(0, Some(max), None);
        assert!(empty.is_empty());
        let mut far_low = Octagon::new(1);
        far_low.add_exact(0, Some(max), None);
        assert!(!far_low.is_empty());
        far_low.add_lin(&[x(0)], 10);
        assert!(far_low.is_empty(), "the matrix's bound below the far one");
        // A join keeps a far bound both sides hold, the weaker.
        let mut other = Octagon::new(2);
        other.add_exact(0, None, Some(max));
        let joined = o.join(&other);
        assert_eq!(joined.far_bounds(0), (None, Some(max)));
        assert_eq!(o.join(&Octagon::new(2)).far_bounds(0), (None, None));
        // Forgetting a variable forgets its far bounds, and a table emptied equals none.
        let mut forgot = o.clone();
        forgot.forget(0);
        assert_eq!(forgot.far_bounds(0), (None, None));
        let mut fresh = Octagon::new(2);
        fresh.add_lin(&[nx(0)], 0);
        assert_eq!(forgot.far_constraints(), Vec::new());
        assert_ne!(forgot, o);
        let mut plain = Octagon::new(2);
        plain.forget(0);
        assert_eq!(forgot, plain);
        // A valuation respects a far bound an i128 holds.
        let mut low = Octagon::new(1);
        low.add_exact(0, Some(Exact::from_i128(BIG + 5)), None);
        assert_eq!(low.pick(&[0]), Some(vec![Exact::from_i128(BIG + 5)]));
        // Within reach, a bound is the matrix's.
        let mut near = Octagon::new(1);
        near.add_exact(0, Some(Exact::from_i128(-3)), Some(Exact::from_i128(7)));
        assert_eq!(near.bounds(0), (-3, 7));
        assert_eq!(near.far_bounds(0), (None, None));
    }

    /// Past [`RELATIONAL_LIMIT`] variables an octagon is a box: each variable's bounds alone,
    /// which a constraint on two variables narrows by the other's bounds, once.
    #[test]
    fn test_past_the_limit_an_octagon_keeps_bounds_alone() {
        let n = RELATIONAL_LIMIT + 1;
        assert!(Octagon::new(RELATIONAL_LIMIT).is_relational());
        let mut b = Octagon::new(n);
        assert!(!b.is_relational());
        b.add_lin(&[x(0)], 5); // a <= 5
        b.add_lin(&[nx(0)], 0); // a >= 0
        b.add_lin(&[x(1), nx(0)], 0); // b - a <= 0, so b <= 5
        assert_eq!((b.bounds(0), b.bounds(1)), ((0, 5), (-INF, 5)));
        // The relation itself is gone: `b - a` is bounded by their bounds alone, where an
        // octagon keeps it.
        assert_eq!(b.upper(&[x(1), nx(0)]), 5);
        let mut o = Octagon::new(2);
        o.add_lin(&[x(0)], 5);
        o.add_lin(&[nx(0)], 0);
        o.add_lin(&[x(1), nx(0)], 0);
        assert_eq!(o.upper(&[x(1), nx(0)]), 0);
        // What it gives the SMT cross-check: each bound on twice a term.
        assert_eq!(
            b.constraints(),
            [
                (vec![x(0), x(0)], 10),
                (vec![nx(0), nx(0)], 0),
                (vec![x(1), x(1)], 10)
            ]
        );
        // Forgetting and joining act on the bounds.
        let mut f = b.clone();
        f.forget(0);
        assert_eq!((f.bounds(0), f.bounds(1)), ((-INF, INF), (-INF, 5)));
        let mut g = Octagon::new(n);
        g.add_lin(&[x(0)], 9);
        g.add_lin(&[nx(0)], -7);
        assert_eq!(b.join(&g).bounds(0), (0, 9));
        assert!(!b.join(&g).is_relational());
        // Of one variable: `c + c <= 8` bounds `c` by 4, and `c - c` is 0.
        let mut h = Octagon::new(n);
        h.add_lin(&[x(2), x(2)], 8);
        assert_eq!(h.bounds(2), (-INF, 4));
        assert_eq!((h.upper(&[x(2), x(2)]), h.upper(&[x(2), nx(2)])), (8, 0));
        h.add_lin(&[x(2), nx(2)], -1);
        assert!(h.is_empty());
        // A weaker bound leaves the stronger one, and true constants change nothing.
        let mut w = b.clone();
        w.add_lin(&[x(0)], 9);
        w.add_lin(&[], 0);
        w.add_lin(&[x(1), nx(1)], 0);
        assert_eq!((w.bounds(0), w.is_empty()), ((0, 5), false));
        // Bounds that cross, or a false constant, empty a box.
        let mut e = b.clone();
        e.add_lin(&[nx(0)], -6);
        assert!(e.is_empty());
        let mut z = b.clone();
        z.add_lin(&[], -1);
        assert!(z.is_empty());
        assert_eq!(z.constraints()[0], (Vec::new(), -1));
    }

    #[test]
    fn test_bounds_and_relations_propagate() {
        let mut o = Octagon::new(3);
        o.add_lin(&[x(0)], 10); // a <= 10
        o.add_lin(&[x(1), nx(0)], -1); // b - a <= -1
        o.add_lin(&[nx(1)], 0); // b >= 0
        assert_eq!(o.bounds(1), (0, 9));
        assert_eq!(o.bounds(0), (1, 10));
        assert!(o.implies(&[x(1), nx(0)], -1));
        assert!(!o.implies(&[x(1), nx(0)], -2));
        assert_eq!(o.upper(&[x(0), x(0)]), 20);
        assert_eq!(o.upper(&[x(0), nx(0)]), 0);
        assert_eq!(o.upper(&[]), 0);
        assert_eq!(o.upper(&[x(0), x(1), x(2)]), INF);
        assert_eq!(o.bounds(2), (-INF, INF));
        assert_eq!(o.vars(), 3);
        // A sum of more than two terms adds nothing.
        let before = o.clone();
        o.add_lin(&[x(0), x(1), x(2)], -100);
        assert_eq!(o, before);
    }

    #[test]
    fn test_integer_tightening_finds_emptiness() {
        // 2a <= 1 and 2a >= 1 has a rational point (1/2) and no integer one.
        let mut o = Octagon::new(1);
        o.add_lin(&[x(0), x(0)], 1);
        assert!(!o.is_empty());
        o.add_lin(&[nx(0), nx(0)], -1);
        assert!(o.is_empty());
        assert!(o.implies(&[x(0)], -5));
        assert_eq!(o.pick(&[0]), None);
        let mut constant = Octagon::new(1);
        constant.add_lin(&[], 0);
        assert!(!constant.is_empty());
        constant.add_lin(&[], -1);
        assert!(constant.is_empty());
        let mut opposite = Octagon::new(1);
        opposite.add_lin(&[x(0), nx(0)], 0);
        assert!(!opposite.is_empty());
        opposite.add_lin(&[x(0), nx(0)], -1);
        assert!(opposite.is_empty());
    }

    #[test]
    fn test_huge_constants_weaken_instead_of_overflowing() {
        let mut o = Octagon::new(2);
        o.add_lin(&[x(0)], BIG);
        assert_eq!(o.bounds(0), (-INF, INF));
        o.add_lin(&[x(0)], BIG - 1);
        assert_eq!(o.bounds(0), (-INF, BIG - 1));
        o.add_lin(&[x(1), nx(0)], i128::MIN / 2);
        assert!(!o.is_empty());
        assert!(o.implies(&[x(1), nx(0)], -BIG));
        assert!(!o.implies(&[x(1), nx(0)], -BIG - 1));
    }

    struct Rng(u64);
    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    fn random_constraint(r: &mut Rng, n: usize) -> (Vec<Term>, i128) {
        let t = |r: &mut Rng| Term {
            var: r.below(n as u64) as usize,
            neg: r.below(2) == 1,
            at: 0,
        };
        let terms = if r.below(3) == 0 {
            vec![t(r)]
        } else {
            vec![t(r), t(r)]
        };
        (terms, r.below(13) as i128 - 6)
    }

    fn satisfies(point: &[i128], terms: &[Term], c: i128) -> bool {
        terms
            .iter()
            .map(|t| if t.neg { -point[t.var] } else { point[t.var] })
            .sum::<i128>()
            <= c
    }

    /// Around a small limit the octagon claims no more than exact arithmetic gives, as a matrix
    /// and as a box. The constants lie on both sides of the limit and of half of it, where a
    /// constraint is weakened, and the points reach past both. Every point that satisfies the
    /// constraints added lies within every bound the octagon reports, of one term or of two,
    /// and within each variable's bounds as intervals hold them; an octagon it calls empty has
    /// no point; the join of two holds the points of both; and one that forgets a variable
    /// holds every point with any value of it.
    #[test]
    fn test_the_octagon_is_sound_around_a_small_limit() {
        const LIMIT: i128 = 8;
        const WINDOW: i128 = 14;
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        let term = |var: usize, neg: bool| Term { var, neg, at: 0 };
        let mut shapes: Vec<Vec<Term>> = Vec::new();
        for a in [false, true] {
            shapes.push(vec![term(0, a)]);
            shapes.push(vec![term(1, a)]);
            for b in [false, true] {
                shapes.push(vec![term(0, a), term(1, b)]);
                shapes.push(vec![term(1, a), term(0, b)]);
            }
        }
        let points = |cs: &[(Vec<Term>, i128)]| -> Vec<[i128; 2]> {
            (-WINDOW..=WINDOW)
                .flat_map(|x| (-WINDOW..=WINDOW).map(move |y| [x, y]))
                .filter(|p| cs.iter().all(|(t, c)| satisfies(p, t, *c)))
                .collect()
        };
        let holds = |o: &Octagon, p: &[i128; 2]| -> bool {
            shapes.iter().all(|t| {
                let u = o.upper(t);
                u >= LIMIT || satisfies(p, t, u)
            }) && (0..2).all(|v| {
                let (lo, hi) = o.bounds(v);
                (lo == -LIMIT || p[v] >= lo) && (hi == LIMIT || p[v] <= hi)
            })
        };
        for vars in [2, RELATIONAL_LIMIT + 1] {
            let mut some = 0;
            for _ in 0..600 {
                let mut made = Vec::new();
                for _ in 0..2 {
                    let mut o = Octagon::within(vars, LIMIT);
                    let mut cs = Vec::new();
                    for _ in 0..(1 + r.below(4)) {
                        let shape = shapes[r.below(shapes.len() as u64) as usize].clone();
                        let c = r.below(25) as i128 - 12;
                        o.add_lin(&shape, c);
                        cs.push((shape, c));
                    }
                    let ps = points(&cs);
                    if o.is_empty() {
                        assert!(
                            ps.is_empty(),
                            "empty, though {:?} satisfies {:?}",
                            ps[0],
                            cs
                        );
                    }
                    for p in &ps {
                        assert!(holds(&o, p), "{:?} satisfies {:?}", p, cs);
                    }
                    made.push((o, cs, ps));
                }
                let (a, ca, pa) = &made[0];
                let (b, cb, pb) = &made[1];
                let joined = a.join(b);
                for p in pa.iter().chain(pb) {
                    assert!(
                        !joined.is_empty() && holds(&joined, p),
                        "{:?} is lost by the join of {:?} and {:?}",
                        p,
                        ca,
                        cb
                    );
                }
                let mut forgot = a.clone();
                forgot.forget(0);
                for p in pa {
                    for x in -WINDOW..=WINDOW {
                        assert!(holds(&forgot, &[x, p[1]]), "{:?} forgets x: {:?}", ca, p);
                    }
                }
                some += usize::from(!pa.is_empty());
            }
            assert!(some > 120, "too few of the octagons hold a point: {}", some);
        }
    }

    /// Whether a matrix is tightly closed: every path no shorter than its edge, every unary
    /// bound even, and every edge no longer than the sum of its two unary bounds' halves.
    fn closed(o: &Octagon) -> bool {
        let d = o.d();
        let at = |i: usize, j: usize| o.at(i, j);
        (0..d).all(|i| {
            (0..d).all(|j| {
                (0..d).all(|k| at(i, j) <= add(INF, at(i, k), at(k, j)))
                    && (at(i, i ^ 1) >= INF || at(i, i ^ 1) % 2 == 0)
                    && (at(i, i ^ 1) >= INF
                        || at(j ^ 1, j) >= INF
                        || at(i, j) <= floor_half(at(i, i ^ 1) + at(j ^ 1, j)))
            })
        })
    }

    #[test]
    fn test_closedness_is_recognised() {
        let mut o = Octagon::new(2);
        o.add_lin(&[x(0)], 3);
        o.add_lin(&[x(1), nx(0)], 1);
        assert!(closed(&o));
        let mut path = o.clone();
        path.put(3, 2, INF);
        assert!(!closed(&path));
        let mut odd = o.clone();
        odd.put(1, 0, 5);
        assert!(!closed(&odd));
        let mut weak = Octagon::new(2);
        weak.add_lin(&[x(0)], 3);
        weak.add_lin(&[x(1)], 4);
        weak.put(1, 2, INF);
        weak.put(3, 0, INF);
        assert!(!closed(&weak));
    }

    /// Against brute force over every integer point of a box: the incremental closure keeps
    /// every point (soundness), finds emptiness exactly, and its bounds are reached by a point;
    /// a picked point satisfies every constraint and lies as close to zero as it can.
    #[test]
    fn test_incremental_closure_agrees_with_brute_force() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let n = 3;
        for _ in 0..300 {
            let mut o = Octagon::new(n);
            let mut cs = Vec::new();
            for v in 0..n {
                cs.push((vec![x(v)], 5));
                cs.push((vec![nx(v)], 5));
            }
            for _ in 0..(1 + r.below(5)) {
                cs.push(random_constraint(&mut r, n));
            }
            for (t, c) in &cs {
                o.add_lin(t, *c);
            }
            let mut points = Vec::new();
            for a in -5..=5 {
                for b in -5..=5 {
                    for c in -5..=5 {
                        let p = [a, b, c];
                        if cs.iter().all(|(t, k)| satisfies(&p, t, *k)) {
                            points.push(p);
                        }
                    }
                }
            }
            assert_eq!(o.is_empty(), points.is_empty(), "{cs:?}");
            if points.is_empty() {
                continue;
            }
            for v in 0..n {
                let (lo, hi) = o.bounds(v);
                assert_eq!(lo, points.iter().map(|p| p[v]).min().unwrap(), "{cs:?}");
                assert_eq!(hi, points.iter().map(|p| p[v]).max().unwrap(), "{cs:?}");
            }
            assert!(closed(&o), "the incremental closure is not closed: {cs:?}");
            let picked: Vec<i128> = o
                .pick(&[0, 1, 2])
                .unwrap()
                .into_iter()
                .map(|v| v.to_i128().unwrap())
                .collect();
            assert!(
                cs.iter().all(|(t, k)| satisfies(&picked, t, *k)),
                "{picked:?} {cs:?}"
            );
            let nearest = points.iter().map(|p| p[0].abs()).min().unwrap();
            assert_eq!(picked[0].abs(), nearest, "{picked:?} {cs:?}");
        }
    }

    /// The constraints an octagon lists hold exactly its points, so that a solver given them is
    /// given the state: against brute force over a box a little wider than the octagon's, and
    /// rebuilt into the same matrix. An empty octagon lists a contradiction.
    #[test]
    fn test_constraints_hold_exactly_the_points_of_the_octagon() {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        let n = 3;
        for _ in 0..300 {
            let mut o = Octagon::new(n);
            let mut cs = Vec::new();
            for v in 0..n {
                cs.push((vec![x(v)], 5));
                cs.push((vec![nx(v)], 5));
            }
            for _ in 0..(1 + r.below(5)) {
                cs.push(random_constraint(&mut r, n));
            }
            for (t, c) in &cs {
                o.add_lin(t, *c);
            }
            let listed = o.constraints();
            for a in -6..=6 {
                for b in -6..=6 {
                    for c in -6..=6 {
                        let p = [a, b, c];
                        let holds = |all: &[(Vec<Term>, i128)]| {
                            all.iter().all(|(t, k)| satisfies(&p, t, *k))
                        };
                        assert_eq!(holds(&cs), holds(&listed), "{p:?} {cs:?} {listed:?}");
                    }
                }
            }
            if o.is_empty() {
                assert!(listed.iter().any(|(t, c)| t.is_empty() && *c < 0), "{cs:?}");
                continue;
            }
            // No constant, and each bound once: a coherent pair holds the same terms in either
            // order.
            let mut seen: Vec<(Vec<(usize, bool)>, i128)> = Vec::new();
            for (t, c) in &listed {
                assert!(!t.is_empty(), "{listed:?}");
                let mut key: Vec<(usize, bool)> = t.iter().map(|t| (t.var, t.neg)).collect();
                key.sort_unstable();
                assert!(!seen.contains(&(key.clone(), *c)), "{listed:?}");
                seen.push((key, *c));
            }
            let mut rebuilt = Octagon::new(n);
            for (t, c) in &listed {
                rebuilt.add_lin(t, *c);
            }
            assert_eq!(rebuilt, o, "{cs:?}");
        }
        assert_eq!(Octagon::new(2).constraints(), Vec::new());
        let mut o = Octagon::new(2);
        o.add_lin(&[x(0), nx(1)], 3);
        assert_eq!(o.constraints(), vec![(vec![nx(1), x(0)], 3)]);
        o.add_lin(&[x(1)], 4);
        assert!(o.constraints().contains(&(vec![x(1), x(1)], 8)));
        assert!(o.constraints().contains(&(vec![x(0), x(0)], 14)));
    }

    /// A valuation takes a bound beyond the matrix's reach exactly, above `i128::MAX` and below
    /// `-BIG` too, never at the matrix's sentinels, and keeps each relation the matrix holds
    /// with a variable so bounded, of a difference or of a sum.
    #[test]
    fn test_pick_respects_far_bounds_and_their_relations() {
        let big = Exact::new(false, 3 * 10u128.pow(38));
        let max = Exact::new(false, u128::MAX);
        let at = |e: Exact, d: i128| e.plus(d).unwrap();
        let mut o = Octagon::new(5);
        o.add_exact(0, Some(big), Some(max));
        // `x1 < x0` with `x1 >= big + 10`, `x2 > x0`, and `x3` related to nothing in `0..=5`.
        o.add_lin(&[x(1), nx(0)], -1);
        o.add_exact(1, Some(at(big, 10)), Some(max));
        o.add_lin(&[x(0), nx(2)], -1);
        o.add_exact(2, Some(Exact::from_i128(0)), Some(max));
        o.add_lin(&[nx(3)], 0);
        o.add_lin(&[x(3)], 5);
        // `x4 = -x0`, as two sums.
        o.add_lin(&[x(4), x(0)], 0);
        o.add_lin(&[nx(4), nx(0)], 0);
        assert_eq!(
            o.pick(&[0, 1, 2, 3, 4]),
            Some(vec![
                at(big, 11),
                at(big, 10),
                at(big, 12),
                Exact::from_i128(0),
                at(big, 11).negated()
            ])
        );
        // The other way round: the sum's variable first.
        assert_eq!(
            o.pick(&[4, 0]),
            Some(vec![at(big, 11).negated(), at(big, 11)])
        );
        // Below `-BIG`, whose lower bound is the matrix's sentinel.
        let mut low = Octagon::new(1);
        let neg = Exact::new(true, 10u128.pow(38));
        low.add_exact(0, Some(Exact::new(true, 1 << 127)), Some(neg));
        assert_eq!(low.pick(&[0]), Some(vec![neg]));
    }

    #[test]
    fn test_pick_prefers_zero_then_the_nearer_bound() {
        let mut o = Octagon::new(3);
        o.add_lin(&[nx(0)], -3); // a >= 3
        o.add_lin(&[x(1)], -2); // b <= -2
        o.add_lin(&[x(2), nx(0)], 0); // c <= a
        assert_eq!(
            o.pick(&[0, 1, 2]),
            Some(vec![3, -2, 0].into_iter().map(Exact::from_i128).collect())
        );
        assert_eq!(o.pick(&[]), Some(vec![]));
    }

    #[test]
    fn test_join_and_forget() {
        let mut a = Octagon::new(2);
        a.add_lin(&[x(0)], 1);
        a.add_lin(&[nx(0)], -1);
        let mut b = Octagon::new(2);
        b.add_lin(&[x(0)], 4);
        b.add_lin(&[nx(0)], -3);
        assert_eq!(a.join(&b).bounds(0), (1, 4));
        let mut gone = b.clone();
        gone.set_empty();
        assert_eq!(a.join(&gone), a);
        assert_eq!(gone.join(&a), a);
        let mut c = a.clone();
        c.add_lin(&[x(1), nx(0)], 0);
        c.forget(0);
        assert_eq!(c.bounds(0), (-INF, INF));
        assert_eq!(c.bounds(1), (-INF, 1));
        // A bound on one variable says nothing of its sum with an unbounded one.
        let mut one = Octagon::new(2);
        one.add_lin(&[x(0)], -3);
        assert_eq!(one.upper(&[x(1), x(0)]), INF);
        assert_eq!(one.upper(&[x(0), x(1)]), INF);
        assert_eq!(one.upper(&[nx(1), x(0)]), INF);
        assert_eq!(one.bounds(0), (-INF, -3));
    }

    /// Each variable's stamp is the evaluation whose value it holds: a join keeps one held on
    /// both sides, or on one beside none, and marks two `MIXED`; forgetting a variable clears
    /// it; narrowing one no evaluation holds stamps it with the term's; a replay rebases what a
    /// loop left as it was entered, and keeps two joined; and neither the bounds nor equality
    /// read the stamps.
    #[test]
    fn test_stamps_follow_the_evaluations_a_state_holds() {
        let mut a = Octagon::new(3);
        a.stamp(0, 4);
        a.stamp(1, 5);
        let mut b = Octagon::new(3);
        b.stamp(0, 4);
        b.stamp(1, 6);
        b.stamp(2, 7);
        let joined = a.join(&b);
        let stamps = |o: &Octagon| (o.stamp_of(0), o.stamp_of(1), o.stamp_of(2));
        assert_eq!(stamps(&joined), (4, MIXED, 7));
        assert_eq!(stamps(&a.join(&Octagon::new(3))), (4, 5, 0));
        assert_eq!(Octagon::new(3).join(&Octagon::new(3)).stamps, None);
        assert_eq!(a, Octagon::new(3));
        let mut forgot = a.clone();
        forgot.forget(0);
        assert_eq!(stamps(&forgot), (0, 5, 0));
        let mut unstamped = Octagon::new(1);
        unstamped.stamp(0, 0);
        assert_eq!(unstamped.stamps, None);
        // A term of the evaluation held is read; one narrowing a variable none holds stamps it.
        let mut n = Octagon::new(2);
        n.add_lin(&[Term::of(0, 9)], 3);
        assert_eq!(n.stamp_of(0), 9);
        assert_eq!(n.upper(&[Term::of(0, 9)]), 3);
        assert!(n.implies(&[Term::of(0, 9)], 3));
        n.add_exact_of(Term::of(1, 2), None, Some(Exact::from_i128(BIG + 1)));
        assert_eq!(n.stamp_of(1), 2);
        assert_eq!(
            n.far_bounds_of(Term::of(1, 2)),
            (None, Some(Exact::from_i128(BIG + 1)))
        );
        // A replay at another entry: the evaluation carried through is the new entry's, none
        // where that entry holds none, and what the loop gave or joined stays.
        let mut exit = a.clone();
        exit.stamp(2, 8);
        let mut to = Octagon::new(3);
        to.stamp(0, 11);
        exit.restamp(&a, &to);
        assert_eq!(stamps(&exit), (11, 0, 8));
        let mut mixed = joined.clone();
        to.stamp(1, 12);
        mixed.restamp(&joined, &to);
        assert_eq!(stamps(&mixed), (11, MIXED, 0));
        // A write that forgets first stamps anew; one that stamps the evaluation held again is
        // no write.
        let mut write = a.clone();
        write.forget(1);
        write.stamp(1, 13);
        write.stamp(1, 13);
        assert_eq!(write.stamp_of(1), 13);
    }

    /// A write that stamps a variable still holding an earlier evaluation stops the compile:
    /// the writer that skips its forget, planted.
    #[test]
    #[should_panic(
        expected = "internal error: the value of `x % 4` is given a new evaluation where the state holds an earlier one, which its write did not forget"
    )]
    fn test_a_write_that_keeps_an_earlier_evaluation_stops_the_compile() {
        let mut o = Octagon::new(1);
        o.name(Arc::from(vec!["the value of `x % 4`".to_string()]));
        o.stamp(0, 4);
        o.add_lin(&[Term::of(0, 4)], 3);
        o.stamp(0, 5);
    }

    /// A term read as an evaluation the state does not hold stops the compile, naming the
    /// variable: another evaluation, two joined, or one the term does not name.
    #[test]
    #[should_panic(
        expected = "internal error: the value of `x % 4` is read as one evaluation, where the state holds another (ghost-contracts-and-totality.md, item 2, no-site-value-from-another-evaluation)"
    )]
    fn test_a_term_of_another_evaluation_stops_the_compile() {
        let mut o = Octagon::new(1);
        o.name(Arc::from(vec!["the value of `x % 4`".to_string()]));
        o.stamp(0, 4);
        o.upper(&[Term::of(0, 5)]);
    }

    #[test]
    #[should_panic(
        expected = "internal error: variable 0 is read as one evaluation, where the state holds two joined"
    )]
    fn test_a_term_where_two_evaluations_are_joined_stops_the_compile() {
        let mut a = Octagon::new(1);
        a.stamp(0, 4);
        let mut b = Octagon::new(1);
        b.stamp(0, 5);
        a.join(&b).implies(&[Term::of(0, 4)], 0);
    }

    #[test]
    #[should_panic(
        expected = "internal error: variable 0 is read with no evaluation named, where the state holds one"
    )]
    fn test_a_term_naming_no_evaluation_stops_the_compile() {
        let mut o = Octagon::new(1);
        o.stamp(0, 4);
        o.add_lin(&[Term::pos(0)], 1);
    }

    #[test]
    #[should_panic(
        expected = "internal error: variable 0 is read as one evaluation, where the state holds another"
    )]
    fn test_a_far_bound_of_another_evaluation_stops_the_compile() {
        let mut o = Octagon::new(1);
        o.stamp(0, 4);
        o.far_bounds_of(Term::of(0, 5));
    }
}

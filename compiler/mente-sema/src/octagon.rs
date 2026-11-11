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

/// The most variables an octagon relates; past them it is a box. The largest function of the
/// repository's corpus has 53 (Norman's decision, `tier1-variable-cap`).
pub const RELATIONAL_LIMIT: usize = 128;

/// No bound. Sums saturate here; a bound at or beyond it is no bound, so the 128-bit types,
/// whose ranges reach past it, are unbounded in the domain.
pub const INF: i128 = i128::MAX / 4;

/// The largest magnitude a constraint's constant keeps: beyond it a constraint is weakened,
/// which is sound, so that doubling a constant never overflows.
pub const BIG: i128 = INF / 2;

/// A sum of two bounds, kept within `[-INF, INF]`: at or beyond `INF` it is no bound, and below
/// `-INF` it is raised to `-INF`, a weaker bound, which is sound and keeps every later sum of
/// two bounds inside `i128`.
fn add(a: i128, b: i128) -> i128 {
    if a >= INF || b >= INF {
        INF
    } else {
        (a + b).clamp(-INF, INF)
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

/// One term of a linear expression: `+x` or `-x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Term {
    pub var: usize,
    pub neg: bool,
}

impl Term {
    pub fn pos(var: usize) -> Term {
        Term { var, neg: false }
    }

    pub fn negated(self) -> Term {
        Term {
            var: self.var,
            neg: !self.neg,
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
}

/// A variable's lower and upper bounds beyond the matrix's reach.
type FarBounds = (Option<Exact>, Option<Exact>);

/// Two octagons are equal when they hold the same bounds: a table of far bounds made but empty
/// is no table.
impl PartialEq for Octagon {
    fn eq(&self, other: &Octagon) -> bool {
        self.n == other.n
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
        let d = 2 * n;
        if n > RELATIONAL_LIMIT {
            return Octagon {
                n,
                m: vec![INF; d],
                boxed: true,
                empty: false,
                far: None,
            };
        }
        let mut m = vec![INF; d * d];
        for k in 0..d {
            m[k * d + k] = 0;
        }
        Octagon {
            n,
            m,
            boxed: false,
            empty: false,
            far: None,
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
                Some(c) => self.add_lin(&[Term::pos(var).negated()], -c),
                None => {
                    let kept = &mut self.far_mut()[var].0;
                    *kept = Some(kept.map_or(l, |k| k.max(l)));
                }
            }
        }
        if let Some(u) = upper {
            match u.to_i128().filter(|_| !u.is_far()) {
                Some(c) => self.add_lin(&[Term::pos(var)], c),
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

    /// The bounds of `var`, exactly, from the matrix and beyond it; `None` for none.
    pub fn exact_bounds(&self, var: usize) -> (Option<Exact>, Option<Exact>) {
        let (lo, hi) = self.bounds(var);
        let (far_lo, far_hi) = self.far_of(var);
        let lo = (lo > -INF).then(|| Exact::from_i128(lo));
        let hi = (hi < INF).then(|| Exact::from_i128(hi));
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
        if v >= INF || v >= self.m[a] {
            return;
        }
        self.m[a] = (2 * floor_half(v)).max(-INF);
        let k = a & !1;
        if add(self.m[k], self.m[k + 1]) < 0 {
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
                    let v = add(c, self.upper(&[b.negated()]));
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
            let a = add(*ki, c);
            for (l, jl) in from_j.iter().enumerate() {
                let v = add(a, *jl).min(self.at(k, l));
                self.put(k, l, v);
            }
        }
    }

    fn tighten_and_strengthen(&mut self) {
        let d = self.d();
        for k in 0..d {
            let v = self.at(k, k ^ 1);
            if v < INF {
                self.put(k, k ^ 1, (2 * floor_half(v)).max(-INF));
            }
        }
        for i in 0..d {
            let ci = self.at(i, i ^ 1);
            if ci >= INF {
                continue;
            }
            for j in 0..d {
                let cj = self.at(j ^ 1, j);
                if cj >= INF {
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
    /// constant beyond [`BIG`] is weakened to no bound, or to `-BIG`.
    pub fn add_lin(&mut self, terms: &[Term], c: i128) {
        if c >= BIG || self.empty {
            return;
        }
        let c = c.max(-BIG);
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

    /// The least upper bound of `terms` (at most two), or `INF`.
    pub fn upper(&self, terms: &[Term]) -> i128 {
        match terms {
            [] => 0,
            [t] => {
                let v = self.unary(t.node());
                if v >= INF {
                    INF
                } else {
                    floor_half(v)
                }
            }
            // A box relates nothing: two terms are at most the sum of each one's bound, or, of
            // one variable, twice it or nothing.
            [s, t] if self.boxed => match (s.var == t.var, s.neg == t.neg) {
                (true, true) => self.unary(s.node()),
                (true, false) => 0,
                (false, _) => add(self.upper(&[*s]), self.upper(&[*t])),
            },
            [s, t] => self.at(t.node() ^ 1, s.node()),
            _ => INF,
        }
    }

    /// Whether `terms <= c` holds in every point of the octagon.
    pub fn implies(&self, terms: &[Term], c: i128) -> bool {
        self.empty || self.upper(terms) <= c
    }

    /// The bounds of one variable, with `-INF` and `INF` for none.
    pub fn bounds(&self, var: usize) -> (i128, i128) {
        let hi = self.upper(&[Term::pos(var)]);
        let lo = self.upper(&[Term::pos(var).negated()]);
        (if lo >= INF { -INF } else { -lo }, hi)
    }

    /// Forgets everything known about `var`.
    pub fn forget(&mut self, var: usize) {
        if let Some(f) = &mut self.far {
            f[var] = (None, None);
        }
        if self.boxed {
            self.m[2 * var] = INF;
            self.m[2 * var + 1] = INF;
            return;
        }
        let d = self.d();
        for node in [2 * var, 2 * var + 1] {
            for k in 0..d {
                if k != node {
                    self.put(node, k, INF);
                    self.put(k, node, INF);
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
        };
        let mut out = Vec::new();
        if self.boxed {
            // Each node's bound on twice its term, and an empty box as a false constant.
            if self.empty {
                out.push((Vec::new(), -1));
            }
            for a in 0..d {
                if self.m[a] < INF {
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
                } else if c < INF {
                    // `V_j - V_i <= c`, and `-V_i` is the node `i ^ 1`.
                    out.push((vec![term(j), term(i ^ 1)], c));
                }
            }
        }
        out
    }

    /// A point of the octagon: each of `vars` in turn takes the value within its bounds that
    /// is closest to zero, which is then fixed, so the result satisfies every constraint.
    pub fn pick(&self, vars: &[usize]) -> Option<Vec<i128>> {
        if self.empty {
            return None;
        }
        let mut s = self.clone();
        let mut out = Vec::new();
        for &v in vars {
            let (mut lo, mut hi) = s.bounds(v);
            // A bound beyond the matrix that an `i128` holds narrows the choice too.
            let (far_lo, far_hi) = s.far_of(v);
            if let Some(l) = far_lo.and_then(Exact::to_i128) {
                lo = lo.max(l);
            }
            if let Some(u) = far_hi.and_then(Exact::to_i128) {
                hi = hi.min(u);
            }
            let value = 0.clamp(lo, hi.max(lo));
            s.add_lin(&[Term::pos(v)], value);
            s.add_lin(&[Term::pos(v).negated()], -value);
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
        Term { var: v, neg: true }
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
        assert_eq!(low.pick(&[0]), Some(vec![BIG + 5]));
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

    /// Whether a matrix is tightly closed: every path no shorter than its edge, every unary
    /// bound even, and every edge no longer than the sum of its two unary bounds' halves.
    fn closed(o: &Octagon) -> bool {
        let d = o.d();
        let at = |i: usize, j: usize| o.at(i, j);
        (0..d).all(|i| {
            (0..d).all(|j| {
                (0..d).all(|k| at(i, j) <= add(at(i, k), at(k, j)))
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
            let picked = o.pick(&[0, 1, 2]).unwrap();
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

    #[test]
    fn test_pick_prefers_zero_then_the_nearer_bound() {
        let mut o = Octagon::new(3);
        o.add_lin(&[nx(0)], -3); // a >= 3
        o.add_lin(&[x(1)], -2); // b <= -2
        o.add_lin(&[x(2), nx(0)], 0); // c <= a
        assert_eq!(o.pick(&[0, 1, 2]), Some(vec![3, -2, 0]));
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
}

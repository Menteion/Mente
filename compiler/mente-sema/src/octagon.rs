//! The octagon domain over integers: a difference-bound matrix over `+x` and `-x` for each
//! variable (`spec/ghost-contracts-and-totality.md`, item 4), kept tightly closed. Ported from
//! the M0b spike (`spikes/tier1-latency/src/octagon.rs`).
//!
//! Entry `m[i][j]` bounds `V_j - V_i`, where `V_2k = +x_k` and `V_2k+1 = -x_k`. Adding one
//! constraint costs O(n^2): a shortest-path relaxation for the new edge and its coherent twin,
//! then tightening and strengthening (Mine; Bagnara, Hill and Zaffanella for integers).

/// No bound. Sums saturate here; a bound at or beyond it is no bound, so the 128-bit types,
/// whose ranges reach past it, are unbounded in the domain.
pub const INF: i128 = i128::MAX / 4;

/// The largest magnitude a constraint's constant keeps: beyond it a constraint is weakened,
/// which is sound, so that doubling a constant never overflows.
pub const BIG: i128 = INF / 2;

fn add(a: i128, b: i128) -> i128 {
    if a >= INF || b >= INF {
        INF
    } else {
        (a + b).min(INF)
    }
}

fn floor_half(v: i128) -> i128 {
    v.div_euclid(2)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Octagon {
    n: usize,
    m: Vec<i128>,
    empty: bool,
}

impl Octagon {
    /// The octagon of `n` unconstrained variables.
    pub fn new(n: usize) -> Octagon {
        let d = 2 * n;
        let mut m = vec![INF; d * d];
        for k in 0..d {
            m[k * d + k] = 0;
        }
        Octagon { n, m, empty: false }
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
                let v = add(a, *jl);
                if v < self.at(k, l) {
                    self.put(k, l, v);
                }
            }
        }
    }

    fn tighten_and_strengthen(&mut self) {
        let d = self.d();
        for k in 0..d {
            let v = self.at(k, k ^ 1);
            if v < INF {
                self.put(k, k ^ 1, 2 * floor_half(v));
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
                let s = floor_half(ci + cj);
                if s < self.at(i, j) {
                    self.put(i, j, s);
                }
            }
        }
        if (0..d).any(|k| self.at(k, k) < 0) {
            self.empty = true;
        }
    }

    /// Recomputes the tight closure from scratch (Floyd-Warshall, then tightening and
    /// strengthening). The incremental path never needs it; the tests compare the two.
    pub fn close(&mut self) {
        let d = self.d();
        for k in 0..d {
            for i in 0..d {
                let ik = self.at(i, k);
                for j in 0..d {
                    let v = add(ik, self.at(k, j));
                    if v < self.at(i, j) {
                        self.put(i, j, v);
                    }
                }
            }
        }
        self.tighten_and_strengthen();
    }

    /// Adds `terms <= c`, for at most two terms; a longer sum is ignored, which is sound. A
    /// constant beyond [`BIG`] is weakened to no bound, or to `-BIG`.
    pub fn add_lin(&mut self, terms: &[Term], c: i128) {
        if c >= BIG {
            return;
        }
        let c = c.max(-BIG);
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
            [s, t] if s.var == t.var => {
                if s.neg == t.neg {
                    let a = s.node();
                    self.add_edge(a ^ 1, a, c);
                } else if c < 0 {
                    self.empty = true;
                }
            }
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
                let a = t.node();
                let v = self.at(a ^ 1, a);
                if v >= INF {
                    INF
                } else {
                    floor_half(v)
                }
            }
            [s, t] if s.var == t.var => {
                if s.neg == t.neg {
                    self.at(s.node() ^ 1, s.node())
                } else {
                    0
                }
            }
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
        Octagon {
            n: self.n,
            m,
            empty: false,
        }
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
            let (lo, hi) = s.bounds(v);
            let value = 0.clamp(lo, hi);
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
            let mut full = o.clone();
            full.close();
            assert_eq!(full, o, "the incremental closure is not closed: {cs:?}");
            let picked = o.pick(&[0, 1, 2]).unwrap();
            assert!(
                cs.iter().all(|(t, k)| satisfies(&picked, t, *k)),
                "{picked:?} {cs:?}"
            );
            let nearest = points.iter().map(|p| p[0].abs()).min().unwrap();
            assert_eq!(picked[0].abs(), nearest, "{picked:?} {cs:?}");
        }
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
    }
}

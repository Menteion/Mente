//! The arithmetic of Tier 1's intervals, with the limit of its domain as a parameter
//! (`ghost-contracts-and-totality.md`, Engineering Guardrail #1).
//!
//! An interval is a pair `(lo, hi)` of bounds within a limit `inf`:
//!
//! - `lo == -inf` says nothing bounds the value below, and `hi == inf` that nothing bounds it
//!   above. A bound at the limit on its own side is never a value.
//! - Any other bound is a value strictly inside the limit, and the interval holds every integer
//!   from `lo` to `hi`, those beyond the limit included: `(3, inf)` holds every integer from 3
//!   up.
//! - `lo > hi` holds no integer: the value of an operation that always traps.
//!
//! A bound an operation computes at or beyond the limit is weakened to the nearest one the
//! domain holds, which is sound: a lower bound at or above `inf` becomes `inf - 1`, and an upper
//! bound at or below `-inf` becomes `1 - inf`. So a product whose least value is `2 * inf` is
//! known to be at least `inf - 1`, never "at least infinity", and what is computed from it stays
//! a statement about values the program can have. Every operation reads its operands this way
//! first ([`held`]), so that a bound some other code left saturated at the wrong end is still
//! read soundly.
//!
//! The tests run every operation with a limit of 5 over every pair of intervals whose bounds lie
//! within the limit or at it, on either side, against exact arithmetic on every value of a
//! window three times as wide.

/// An interval: its lower and upper bounds.
pub type Itv = (i128, i128);

/// The lower bound the domain keeps for one computed as `x`: none at or below `-inf`, and the
/// greatest it holds at or above `inf`.
pub fn lower(inf: i128, x: i128) -> i128 {
    if x <= -inf {
        -inf
    } else if x >= inf {
        inf - 1
    } else {
        x
    }
}

/// The upper bound the domain keeps for one computed as `x`: none at or above `inf`, and the
/// least it holds at or below `-inf`.
pub fn upper(inf: i128, x: i128) -> i128 {
    if x >= inf {
        inf
    } else if x <= -inf {
        1 - inf
    } else {
        x
    }
}

/// `a` as the domain holds it: each bound a value inside the limit, or none.
pub fn held(inf: i128, a: Itv) -> Itv {
    (lower(inf, a.0), upper(inf, a.1))
}

/// Whether `a` holds no integer.
pub fn is_empty(a: Itv) -> bool {
    a.0 > a.1
}

/// `x + y` for `x` in `a` and `y` in `b`.
pub fn add(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    let lo = if a.0 == -inf || b.0 == -inf {
        -inf
    } else {
        lower(inf, a.0 + b.0)
    };
    let hi = if a.1 == inf || b.1 == inf {
        inf
    } else {
        upper(inf, a.1 + b.1)
    };
    (lo, hi)
}

/// `-x` for `x` in `a`: a bound at the limit negated is the other side's limit, no bound
/// there either.
pub fn neg(inf: i128, a: Itv) -> Itv {
    let a = held(inf, a);
    (-a.1, -a.0)
}

/// `x * y` for `x` in `a` and `y` in `b`: between the least and the greatest product of their
/// bounds. A bound at the limit, no bound on its side, times any value but zero is at or past
/// the limit on the side the signs give, which [`lower`] and [`upper`] read as no bound there,
/// and times zero is zero, as every value times zero is; so it needs no case of its own.
pub fn mul(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    if is_empty(a) || is_empty(b) {
        return (1, 0);
    }
    let products = [
        a.0.saturating_mul(b.0),
        a.0.saturating_mul(b.1),
        a.1.saturating_mul(b.0),
        a.1.saturating_mul(b.1),
    ];
    let least = products.iter().copied().min().unwrap_or(0);
    let most = products.iter().copied().max().unwrap_or(0);
    (lower(inf, least), upper(inf, most))
}

/// `x / y`, rounded toward zero, for `x` in `a` and `y` in `b`. A divisor that is zero traps, so
/// where the division runs `y` is not zero: a range that only touches zero ends at 1 or at -1
/// there, and the quotient keeps the sign that range gives it. A divisor of either sign, or one
/// that is always zero, tells of the quotient only that it is no larger than the dividend.
pub fn div(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    // A divisor of no value, after an operation that always traps, gives a quotient of none.
    if is_empty(b) {
        return b;
    }
    let (c, d) = match b {
        (0, d) if d > 0 => (1, d),
        (c, 0) if c < 0 => (c, -1),
        other => other,
    };
    if c > 0 {
        // A positive divisor keeps the dividend's sign: a dividend that is not negative is
        // least over the greatest divisor and greatest over the least, and a negative one the
        // other way round. Over a divisor of no upper bound the quotient reaches zero.
        let over = |x: i128, y: i128| if y == inf { 0 } else { x / y };
        let lo = if a.0 == -inf {
            -inf
        } else if a.0 >= 0 {
            over(a.0, d)
        } else {
            a.0 / c
        };
        let hi = if a.1 == inf {
            inf
        } else if a.1 >= 0 {
            a.1 / c
        } else {
            over(a.1, d)
        };
        (lo, hi)
    } else if d < 0 {
        neg(inf, div(inf, a, neg(inf, (c, d))))
    } else {
        // Between the dividend's greatest magnitude and its negation, no bound where that is
        // the limit.
        let m = a.0.abs().max(a.1.abs());
        (-m, m)
    }
}

/// `x - x / d` for `x` in `a` and a divisor `d >= 1`, which grows with `x`: what division by
/// `d` leaves of `x`.
pub fn rest(inf: i128, a: Itv, d: i128) -> Itv {
    let a = held(inf, a);
    (
        if a.0 == -inf { -inf } else { a.0 - a.0 / d },
        if a.1 == inf { inf } else { a.1 - a.1 / d },
    )
}

/// `x % y` for `x` in `a` and `y` in `b`: the sign of `x`, and a magnitude below that of `y`
/// and at most that of `x`. A zero divisor traps, so the magnitude of `y` bounds the
/// remainder's even where `y` may be zero. Where neither magnitude is known, nothing is.
pub fn rem(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    let magnitude = |(lo, hi): Itv| (lo != -inf && hi != inf).then(|| lo.abs().max(hi.abs()));
    let bound = match (magnitude(a), magnitude(b).map(|m| m - 1)) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, y) => y,
    };
    (
        if a.0 >= 0 {
            0
        } else {
            bound.map_or(-inf, |m| -m)
        },
        if a.1 <= 0 { 0 } else { bound.unwrap_or(inf) },
    )
}

/// `x * 2^k` for `x` in `a`, which `x << k` is before it is narrowed.
pub fn shl(inf: i128, a: Itv, k: u32) -> Itv {
    let a = held(inf, a);
    // A bound at the limit times `2^k` stays at or past it, no bound on its side.
    let times = |x: i128| x.saturating_mul(1i128.checked_shl(k).unwrap_or(i128::MAX));
    (lower(inf, times(a.0)), upper(inf, times(a.1)))
}

/// `x >> k`, which rounds toward negative infinity, for `x` in `a`.
pub fn shr(inf: i128, a: Itv, k: u32) -> Itv {
    let a = held(inf, a);
    let by = |x: i128| x.checked_shr(k).unwrap_or(if x < 0 { -1 } else { 0 });
    (
        if a.0 == -inf { -inf } else { by(a.0) },
        if a.1 == inf { inf } else { by(a.1) },
    )
}

/// `x >> k` for `x` in `a` and any `k` that is not negative: between `x` and zero, or -1 for a
/// negative `x`.
pub fn shr_any(inf: i128, a: Itv) -> Itv {
    let a = held(inf, a);
    (a.0.min(0), if a.1 >= 0 { a.1 } else { -1 })
}

/// The integers of both `a` and `b`.
pub fn meet(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    (a.0.max(b.0), a.1.min(b.1))
}

/// The least interval that holds the integers of `a` and those of `b`.
pub fn join(inf: i128, a: Itv, b: Itv) -> Itv {
    let (a, b) = (held(inf, a), held(inf, b));
    if is_empty(a) {
        b
    } else if is_empty(b) {
        a
    } else {
        (a.0.min(b.0), a.1.max(b.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limit the tests run with, and how far past it the values they take reach.
    const INF: i128 = 5;
    const WINDOW: i128 = 3 * INF;

    /// Every interval whose bounds lie within the limit or at it, on either side: a lower
    /// bound of `INF` and an upper bound of `-INF` too, as a producer that saturated a bound at
    /// the wrong end leaves them.
    fn intervals() -> Vec<Itv> {
        (-INF..=INF)
            .flat_map(|lo| (-INF..=INF).map(move |hi| (lo, hi)))
            .collect()
    }

    /// Whether the interval `a` holds `x`: a bound at the limit on its own side is none, and
    /// any other is a value, one at the wrong end included.
    fn holds(a: Itv, x: i128) -> bool {
        (a.0 == -INF || x >= a.0) && (a.1 == INF || x <= a.1)
    }

    /// The integers of the window that `a` holds, which reach past the limit.
    fn values(a: Itv) -> Vec<i128> {
        (-WINDOW..=WINDOW).filter(|&x| holds(a, x)).collect()
    }

    /// Whether each bound of `a` is a value inside the limit, or none on its own side.
    fn well_formed(a: Itv) -> bool {
        (a.0 == -INF || a.0.abs() < INF) && (a.1 == INF || a.1.abs() < INF)
    }

    /// Checks `op` against `exact` on every pair of intervals and every pair of their values:
    /// the result is well formed and holds what exact arithmetic gives, where it gives a value.
    fn sound(
        name: &str,
        op: impl Fn(i128, Itv, Itv) -> Itv,
        exact: impl Fn(i128, i128) -> Option<i128>,
    ) {
        for a in intervals() {
            for b in intervals() {
                let r = op(INF, a, b);
                assert!(
                    is_empty(r) || well_formed(r),
                    "{} {:?} {:?} = {:?}",
                    name,
                    a,
                    b,
                    r
                );
                for x in values(a) {
                    for y in values(b) {
                        if let Some(z) = exact(x, y) {
                            assert!(
                                holds(r, z),
                                "{}: {} and {} give {}, outside {:?}, the result of {:?} and {:?}",
                                name,
                                x,
                                y,
                                z,
                                r,
                                a,
                                b
                            );
                        }
                    }
                }
            }
        }
    }

    /// Every binary operation holds what exact arithmetic gives, on every pair of intervals
    /// around the limit and every pair of values they hold, those past the limit included.
    #[test]
    fn test_binary_operations_hold_every_exact_result_around_the_limit() {
        sound("add", add, |x, y| Some(x + y));
        sound("mul", mul, |x, y| Some(x * y));
        // A product with an operand of no value, after an operation that always traps, has
        // none either.
        for a in [(2, 3), (-INF, INF), (0, 0)] {
            assert!(
                is_empty(mul(INF, (1, 0), a)) && is_empty(mul(INF, a, (1, 0))),
                "{:?}",
                a
            );
        }
        sound("div", div, |x, y| (y != 0).then(|| x / y));
        sound("rem", rem, |x, y| (y != 0).then(|| x % y));
        sound("meet", meet, |x, y| (x == y).then_some(x));
        sound("join", join, |x, _| Some(x));
        sound("join", |inf, a, b| join(inf, b, a), |x, _| Some(x));
    }

    /// Every operation of one operand does too.
    #[test]
    fn test_unary_operations_hold_every_exact_result_around_the_limit() {
        for a in intervals() {
            let check = |name: &str, r: Itv, exact: &dyn Fn(i128) -> i128| {
                assert!(is_empty(r) || well_formed(r), "{} {:?} = {:?}", name, a, r);
                for x in values(a) {
                    let z = exact(x);
                    assert!(
                        holds(r, z),
                        "{}: {} gives {}, outside {:?}, the result of {:?}",
                        name,
                        x,
                        z,
                        r,
                        a
                    );
                }
            };
            check("neg", neg(INF, a), &|x| -x);
            check("held", held(INF, a), &|x| x);
            for k in 0..=4 {
                check("shl", shl(INF, a, k), &|x| x * (1 << k));
                check("shr", shr(INF, a, k), &|x| x >> k);
                check("shr of any amount", shr_any(INF, a), &|x| x >> k);
            }
            // A shift right by the width of `i128` or more leaves the sign alone.
            for k in [127, 128, 200] {
                let floor = |x: i128| if x < 0 { -1 } else { 0 };
                check("shr", shr(INF, a, k), &floor);
            }
            for d in 1..=4 {
                check("rest", rest(INF, a, d), &|x| x - x / d);
            }
        }
    }

    /// A bound beyond the limit is weakened to the nearest the domain holds, never read as no
    /// bound on the other side: the least product of two values at the limit's edge is known
    /// to be at least `INF - 1`, and a quotient of it is bounded from that, not from infinity.
    #[test]
    fn test_a_bound_beyond_the_limit_is_the_nearest_the_domain_holds() {
        assert_eq!(lower(INF, 2 * INF), INF - 1);
        assert_eq!(lower(INF, -2 * INF), -INF);
        assert_eq!(upper(INF, 2 * INF), INF);
        assert_eq!(upper(INF, -2 * INF), 1 - INF);
        assert_eq!(held(INF, (INF, -INF)), (INF - 1, 1 - INF));
        // 4 * 4 is 16, past the limit: at least 4, with no upper bound.
        assert_eq!(mul(INF, (4, 4), (4, 4)), (INF - 1, INF));
        // 16 / 3 is 5: the quotient is at least 4 / 3, not at least the limit.
        assert_eq!(div(INF, mul(INF, (4, 4), (4, 4)), (0, 3)), (1, INF));
        // 16 - y for any y at or above 0 has no lower bound.
        let difference = add(INF, mul(INF, (4, 4), (4, 4)), neg(INF, (0, INF)));
        assert_eq!(difference, (-INF, INF));
        // A sum of two unknown ends is unknown on that side, whatever the other operand.
        assert_eq!(add(INF, (INF, INF), (-INF, -INF)), (-INF, INF));
        // The exact rules inside the limit.
        assert_eq!(add(INF, (1, 2), (-1, 1)), (0, 3));
        assert_eq!(mul(INF, (-2, 1), (1, 2)), (-4, 2));
        assert_eq!(div(INF, (-4, 4), (1, 2)), (-4, 4));
        assert_eq!(div(INF, (2, 4), (0, 2)), (1, 4));
        assert_eq!(div(INF, (2, 4), (-2, 0)), (-4, -1));
        assert_eq!(div(INF, (2, 4), (-2, 2)), (-4, 4));
        assert_eq!(div(INF, (2, 4), (0, 0)), (-4, 4));
        assert_eq!(div(INF, (2, 4), (1, 0)), (1, 0));
        assert_eq!(rem(INF, (-4, 4), (1, 3)), (-2, 2));
        assert_eq!(rem(INF, (0, 4), (-INF, INF)), (0, 4));
        assert_eq!(rem(INF, (-INF, INF), (-INF, INF)), (-INF, INF));
        assert_eq!(rem(INF, (-INF, INF), (2, 3)), (-2, 2));
        assert_eq!(shl(INF, (-1, 1), 2), (-4, 4));
        assert_eq!(shr(INF, (-3, 3), 1), (-2, 1));
        assert_eq!(shr_any(INF, (-3, -2)), (-3, -1));
        assert_eq!(rest(INF, (-4, 4), 2), (-2, 2));
        assert_eq!(meet(INF, (-4, 2), (0, INF)), (0, 2));
        assert_eq!(join(INF, (1, 0), (2, 3)), (2, 3));
        assert_eq!(join(INF, (-4, 2), (0, INF)), (-4, INF));
        assert!(is_empty((1, 0)) && !is_empty((0, 0)));
    }
}

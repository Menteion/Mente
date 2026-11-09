//! Exact integers (`spec/core-calculus.md`, items 2 and 4).
//!
//! An M1 integer is a mathematical integer. [`Int`] holds one as a sign and a 128-bit
//! magnitude, so every value of every M1 type fits, and so does the exact result of one
//! operation on two values of types up to 64 bits wide. An operation whose exact result leaves
//! that window returns `None`, and the interpreter reports it.

use std::cmp::Ordering;
use std::fmt;

use mente_sema::types::IntTy;

/// A mathematical integer within `-(2^128 - 1) ..= 2^128 - 1`. Zero is never negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Int {
    neg: bool,
    mag: u128,
}

impl Int {
    pub const ZERO: Int = Int { neg: false, mag: 0 };

    fn new(neg: bool, mag: u128) -> Int {
        Int {
            neg: neg && mag != 0,
            mag,
        }
    }

    pub fn from_u128(mag: u128) -> Int {
        Int::new(false, mag)
    }

    pub fn from_i128(v: i128) -> Int {
        Int::new(v.is_negative(), v.unsigned_abs())
    }

    pub fn is_negative(self) -> bool {
        self.neg
    }

    pub fn magnitude(self) -> u128 {
        self.mag
    }

    /// The value as a `u128`, when it is one.
    pub fn to_u128(self) -> Option<u128> {
        (!self.neg).then_some(self.mag)
    }

    pub fn negate(self) -> Int {
        Int::new(!self.neg, self.mag)
    }

    pub fn checked_add(self, other: Int) -> Option<Int> {
        if self.neg == other.neg {
            return self
                .mag
                .checked_add(other.mag)
                .map(|m| Int::new(self.neg, m));
        }
        Some(match self.mag.cmp(&other.mag) {
            Ordering::Less => Int::new(other.neg, other.mag - self.mag),
            _ => Int::new(self.neg, self.mag - other.mag),
        })
    }

    pub fn checked_sub(self, other: Int) -> Option<Int> {
        self.checked_add(other.negate())
    }

    pub fn checked_mul(self, other: Int) -> Option<Int> {
        self.mag
            .checked_mul(other.mag)
            .map(|m| Int::new(self.neg != other.neg, m))
    }

    /// Division truncated toward zero; `None` for a zero divisor.
    pub fn checked_div(self, other: Int) -> Option<Int> {
        self.mag
            .checked_div(other.mag)
            .map(|m| Int::new(self.neg != other.neg, m))
    }

    /// The remainder, with the sign of `self`; `None` for a zero divisor.
    pub fn checked_rem(self, other: Int) -> Option<Int> {
        self.mag
            .checked_rem(other.mag)
            .map(|m| Int::new(self.neg, m))
    }

    /// `self · 2^k`, exactly.
    pub fn checked_shl(self, k: u32) -> Option<Int> {
        let factor = 1u128.checked_shl(k)?;
        self.checked_mul(Int::from_u128(factor))
    }

    /// `self / 2^k`, rounded toward negative infinity.
    pub fn shr_floor(self, k: u32) -> Int {
        if k >= 128 {
            return if self.neg {
                Int::from_i128(-1)
            } else {
                Int::ZERO
            };
        }
        let quotient = self.mag >> k;
        if self.neg && self.mag & ((1 << k) - 1) != 0 {
            Int::new(true, quotient + 1)
        } else {
            Int::new(self.neg, quotient)
        }
    }

    /// Whether the value lies in `t`'s range.
    pub fn fits(self, t: IntTy) -> bool {
        if self.neg {
            self.mag <= t.min_magnitude()
        } else {
            self.mag <= t.max()
        }
    }

    /// The two's-complement bits of a value that fits `t`, in its low `t.bits` bits.
    pub fn to_bits(self, t: IntTy) -> u128 {
        let bits = if self.neg {
            self.mag.wrapping_neg()
        } else {
            self.mag
        };
        bits & mask(t)
    }

    /// The value of `t` whose two's-complement bits are the low `t.bits` bits of `bits`.
    pub fn from_bits(bits: u128, t: IntTy) -> Int {
        let bits = bits & mask(t);
        if t.signed && bits >> (t.bits - 1) & 1 == 1 {
            Int::new(true, bits.wrapping_neg() & mask(t))
        } else {
            Int::from_u128(bits)
        }
    }
}

/// The low `t.bits` bits set.
fn mask(t: IntTy) -> u128 {
    match t.bits {
        128 => u128::MAX,
        bits => (1 << bits) - 1,
    }
}

impl Ord for Int {
    fn cmp(&self, other: &Int) -> Ordering {
        match (self.neg, other.neg) {
            (false, false) => self.mag.cmp(&other.mag),
            (true, true) => other.mag.cmp(&self.mag),
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
        }
    }
}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Int) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.neg {
            write!(f, "-{}", self.mag)
        } else {
            write!(f, "{}", self.mag)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(v: i128) -> Int {
        Int::from_i128(v)
    }

    fn t(name: &str) -> IntTy {
        IntTy::named(name).unwrap_or(mente_sema::types::I64)
    }

    #[test]
    fn test_integers_add_subtract_and_multiply_exactly() {
        assert_eq!(i(3).checked_add(i(4)), Some(i(7)));
        assert_eq!(i(3).checked_add(i(-5)), Some(i(-2)));
        assert_eq!(i(-3).checked_add(i(5)), Some(i(2)));
        assert_eq!(i(-3).checked_add(i(-5)), Some(i(-8)));
        assert_eq!(i(5).checked_add(i(-5)), Some(Int::ZERO));
        assert!(!i(5).checked_add(i(-5)).unwrap_or(i(1)).is_negative());
        assert_eq!(i(3).checked_sub(i(10)), Some(i(-7)));
        assert_eq!(i(-4).checked_mul(i(6)), Some(i(-24)));
        assert_eq!(i(-4).checked_mul(i(-6)), Some(i(24)));
        assert_eq!(i(0).checked_mul(i(-6)), Some(Int::ZERO));
        let big = Int::from_u128(u128::MAX);
        assert_eq!(big.checked_add(i(1)), None);
        assert_eq!(big.negate().checked_sub(i(1)), None);
        assert_eq!(big.checked_mul(i(2)), None);
        assert_eq!(big.checked_add(big.negate()), Some(Int::ZERO));
        let product =
            Int::from_u128(u64::MAX as u128).checked_mul(Int::from_u128(u64::MAX as u128));
        assert_eq!(
            product.map(|p| p.magnitude()),
            Some((u64::MAX as u128) * (u64::MAX as u128))
        );
    }

    #[test]
    fn test_division_truncates_and_the_remainder_takes_the_left_sign() {
        assert_eq!(i(7).checked_div(i(2)), Some(i(3)));
        assert_eq!(i(-7).checked_div(i(2)), Some(i(-3)));
        assert_eq!(i(7).checked_div(i(-2)), Some(i(-3)));
        assert_eq!(i(-7).checked_div(i(-2)), Some(i(3)));
        assert_eq!(i(7).checked_rem(i(2)), Some(i(1)));
        assert_eq!(i(-7).checked_rem(i(2)), Some(i(-1)));
        assert_eq!(i(7).checked_rem(i(-2)), Some(i(1)));
        assert_eq!(i(7).checked_div(Int::ZERO), None);
        assert_eq!(i(7).checked_rem(Int::ZERO), None);
        assert_eq!(i(-6).checked_rem(i(3)), Some(Int::ZERO));
    }

    #[test]
    fn test_shifts_multiply_and_floor_divide_by_powers_of_two() {
        assert_eq!(i(3).checked_shl(4), Some(i(48)));
        assert_eq!(i(-3).checked_shl(4), Some(i(-48)));
        assert_eq!(i(1).checked_shl(127).map(|v| v.magnitude()), Some(1 << 127));
        assert_eq!(i(2).checked_shl(127), None);
        assert_eq!(i(1).checked_shl(128), None);
        assert_eq!(i(13).shr_floor(2), i(3));
        assert_eq!(i(-13).shr_floor(2), i(-4));
        assert_eq!(i(-12).shr_floor(2), i(-3));
        assert_eq!(i(-1).shr_floor(5), i(-1));
        assert_eq!(i(5).shr_floor(0), i(5));
        assert_eq!(i(5).shr_floor(200), Int::ZERO);
        assert_eq!(i(-5).shr_floor(200), i(-1));
        assert_eq!(i(-5).shr_floor(128), i(-1));
        assert_eq!(i(5).shr_floor(127), Int::ZERO);
    }

    #[test]
    fn test_values_fit_ranges_and_map_to_twos_complement_bits() {
        assert!(i(255).fits(t("u8")) && !i(256).fits(t("u8")));
        assert!(!i(-1).fits(t("u8")));
        assert!(i(-128).fits(t("i8")) && !i(-129).fits(t("i8")));
        assert!(i(127).fits(t("i8")) && !i(128).fits(t("i8")));
        assert_eq!(i(-1).to_bits(t("i8")), 0xFF);
        assert_eq!(i(-128).to_bits(t("i8")), 0x80);
        assert_eq!(i(5).to_bits(t("u8")), 5);
        assert_eq!(Int::from_bits(0xFF, t("i8")), i(-1));
        assert_eq!(Int::from_bits(0x80, t("i8")), i(-128));
        assert_eq!(Int::from_bits(0x7F, t("i8")), i(127));
        assert_eq!(Int::from_bits(0x1FF, t("u8")), i(255));
        assert_eq!(Int::from_bits(u128::MAX, t("i128")), i(-1));
        assert_eq!(
            Int::from_bits(u128::MAX, t("u128")),
            Int::from_u128(u128::MAX)
        );
        assert_eq!(i(-1).to_bits(t("i128")), u128::MAX);
        assert_eq!(Int::from_bits(1 << 127, t("i128")).magnitude(), 1 << 127);
    }

    #[test]
    fn test_integers_compare_and_print_as_numbers() {
        assert!(i(-5) < i(-2) && i(-2) < i(0) && i(0) < i(3) && i(3) < i(9));
        assert!(i(-2) > i(-5) && i(3) > i(-3));
        assert_eq!(i(-42).to_string(), "-42");
        assert_eq!(i(42).to_string(), "42");
        assert_eq!(i(7).to_u128(), Some(7));
        assert_eq!(i(-7).to_u128(), None);
        assert_eq!(Int::from_i128(i128::MIN).magnitude(), 1 << 127);
    }
}

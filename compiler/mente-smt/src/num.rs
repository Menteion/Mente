//! Exact integers of any size, for the constants of a query and for [`crate::eval`], which reads
//! a query's arithmetic as SMT-LIB defines it: a product of two 128-bit values needs 256 bits,
//! and a query states the bounds of every integer type, `2^128` among them.

use std::cmp::Ordering;
use std::fmt;

/// A mathematical integer: a sign and a magnitude in 32-bit limbs, least significant first,
/// with no limb of zero at the top. Zero is never negative.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Big {
    neg: bool,
    mag: Vec<u32>,
}

impl Big {
    pub fn zero() -> Big {
        Big {
            neg: false,
            mag: Vec::new(),
        }
    }

    fn new(neg: bool, mut mag: Vec<u32>) -> Big {
        while mag.last() == Some(&0) {
            mag.pop();
        }
        Big {
            neg: neg && !mag.is_empty(),
            mag,
        }
    }

    pub fn from_u128(v: u128) -> Big {
        let mag = (0..4).map(|k| (v >> (32 * k)) as u32).collect();
        Big::new(false, mag)
    }

    pub fn from_i128(v: i128) -> Big {
        let m = Big::from_u128(v.unsigned_abs());
        if v < 0 {
            m.neg()
        } else {
            m
        }
    }

    /// `2^k`.
    pub fn pow2(k: u32) -> Big {
        let mut mag = vec![0; (k / 32) as usize + 1];
        mag[(k / 32) as usize] = 1 << (k % 32);
        Big::new(false, mag)
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn is_negative(&self) -> bool {
        self.neg
    }

    /// The value as an `i128`, when it is one.
    pub fn to_i128(&self) -> Option<i128> {
        let m = self.magnitude_u128()?;
        if self.neg {
            if m <= i128::MIN.unsigned_abs() {
                Some(0i128.wrapping_sub_unsigned(m))
            } else {
                None
            }
        } else {
            i128::try_from(m).ok()
        }
    }

    /// The value as a `u128`, when it is one.
    pub fn to_u128(&self) -> Option<u128> {
        if self.neg {
            None
        } else {
            self.magnitude_u128()
        }
    }

    /// The magnitude as a `u128`, when it fits.
    pub fn magnitude_u128(&self) -> Option<u128> {
        if self.mag.len() > 4 {
            return None;
        }
        Some(
            self.mag
                .iter()
                .enumerate()
                .fold(0u128, |acc, (k, &l)| acc | (u128::from(l) << (32 * k))),
        )
    }

    pub fn neg(&self) -> Big {
        Big::new(!self.neg, self.mag.clone())
    }

    pub fn abs(&self) -> Big {
        Big::new(false, self.mag.clone())
    }

    pub fn add(&self, other: &Big) -> Big {
        if self.neg == other.neg {
            return Big::new(self.neg, add_mag(&self.mag, &other.mag));
        }
        match cmp_mag(&self.mag, &other.mag) {
            Ordering::Less => Big::new(other.neg, sub_mag(&other.mag, &self.mag)),
            Ordering::Equal | Ordering::Greater => {
                Big::new(self.neg, sub_mag(&self.mag, &other.mag))
            }
        }
    }

    pub fn sub(&self, other: &Big) -> Big {
        self.add(&other.neg())
    }

    pub fn mul(&self, other: &Big) -> Big {
        let mut out = vec![0u32; self.mag.len() + other.mag.len()];
        for (i, &a) in self.mag.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.mag.iter().enumerate() {
                let t = u64::from(a) * u64::from(b) + u64::from(out[i + j]) + carry;
                out[i + j] = t as u32;
                carry = t >> 32;
            }
            let mut k = i + other.mag.len();
            while carry > 0 {
                let t = u64::from(out[k]) + carry;
                out[k] = t as u32;
                carry = t >> 32;
                k += 1;
            }
        }
        Big::new(self.neg != other.neg, out)
    }

    /// SMT-LIB's `div` and `mod`: for a divisor `n` other than 0, `self = n * q + r` with
    /// `0 <= r < |n|`. `None` for a divisor of 0, which SMT-LIB leaves to the model.
    pub fn div_mod(&self, n: &Big) -> Option<(Big, Big)> {
        if n.is_zero() {
            return None;
        }
        let (q, r) = div_mag(&self.mag, &n.mag);
        // Magnitudes first: |self| = |n| * q + r.
        let (q, r) = (Big::new(false, q), Big::new(false, r));
        if !self.neg {
            // self = |n| q + r, so for a negative n the quotient is -q.
            let q = if n.neg { q.neg() } else { q };
            return Some((q, r));
        }
        if r.is_zero() {
            let q = if n.neg { q } else { q.neg() };
            return Some((q, r));
        }
        // self = -(|n| q + r) = |n| (-q - 1) + (|n| - r), with 0 < |n| - r < |n|.
        let r = n.abs().sub(&r);
        let q = q.add(&Big::from_u128(1));
        let q = if n.neg { q } else { q.neg() };
        Some((q, r))
    }

    /// The value modulo `2^bits`, in `0 .. 2^bits`: its two's-complement bits in `bits` bits.
    pub fn mod_pow2(&self, bits: u32) -> Big {
        let limbs = (bits / 32) as usize;
        let mut mag: Vec<u32> = self.mag.iter().copied().take(limbs + 1).collect();
        mag.resize(limbs + 1, 0);
        mag[limbs] &= (1u32 << (bits % 32)).wrapping_sub(1);
        let low = Big::new(false, mag);
        if self.neg && !low.is_zero() {
            Big::pow2(bits).sub(&low)
        } else {
            low
        }
    }

    /// The value written as SMT-LIB writes a numeral: `5`, `(- 5)`.
    pub fn smt(&self) -> String {
        if self.neg {
            format!("(- {})", self.abs())
        } else {
            self.to_string()
        }
    }

    /// A decimal numeral, with `-` before it or not.
    pub fn parse(text: &str) -> Option<Big> {
        let (neg, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let ten = Big::from_u128(10);
        let mut v = Big::zero();
        for b in digits.bytes() {
            v = v.mul(&ten).add(&Big::from_u128(u128::from(b - b'0')));
        }
        Some(if neg { v.neg() } else { v })
    }
}

impl Ord for Big {
    fn cmp(&self, other: &Big) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &other.mag),
            (true, true) => cmp_mag(&other.mag, &self.mag),
        }
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, other: &Big) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Big {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }
        // Nine decimal digits at a time, least significant first.
        let mut chunks = Vec::new();
        let mut rest = self.mag.clone();
        while !rest.is_empty() {
            let mut rem = 0u64;
            for limb in rest.iter_mut().rev() {
                let cur = (rem << 32) | u64::from(*limb);
                *limb = (cur / 1_000_000_000) as u32;
                rem = cur % 1_000_000_000;
            }
            while rest.last() == Some(&0) {
                rest.pop();
            }
            chunks.push(rem);
        }
        if self.neg {
            write!(f, "-")?;
        }
        let mut chunks = chunks.into_iter().rev();
        if let Some(first) = chunks.next() {
            write!(f, "{}", first)?;
        }
        for c in chunks {
            write!(f, "{:09}", c)?;
        }
        Ok(())
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for k in 0..a.len().max(b.len()) {
        let t = u64::from(a.get(k).copied().unwrap_or(0))
            + u64::from(b.get(k).copied().unwrap_or(0))
            + carry;
        out.push(t as u32);
        carry = t >> 32;
    }
    out.push(carry as u32);
    out
}

/// `a - b` for `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (k, &x) in a.iter().enumerate() {
        let mut t = i64::from(x) - i64::from(b.get(k).copied().unwrap_or(0)) - borrow;
        borrow = 0;
        if t < 0 {
            t += 1 << 32;
            borrow = 1;
        }
        out.push(t as u32);
    }
    out
}

/// The quotient and remainder of two magnitudes, `b` not zero, a bit at a time.
fn div_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut q = vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new();
    for k in (0..a.len() * 32).rev() {
        // r = 2r + bit k of a.
        let mut carry = (a[k / 32] >> (k % 32)) & 1;
        for limb in r.iter_mut() {
            let top = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = top;
        }
        if carry > 0 {
            r.push(carry);
        }
        if cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            while r.last() == Some(&0) {
                r.pop();
            }
            q[k / 32] |= 1 << (k % 32);
        }
    }
    (q, r)
}

#[cfg(test)]
mod tests {
    use super::Big;

    fn b(v: i128) -> Big {
        Big::from_i128(v)
    }

    #[test]
    fn test_big_integers_compute_exactly_beyond_128_bits() {
        let top = Big::from_u128(u128::MAX);
        let square = top.mul(&top);
        assert_eq!(
            square.to_string(),
            "115792089237316195423570985008687907852589419931798687112530834793049593217025"
        );
        assert_eq!(Big::pow2(128).sub(&Big::from_u128(1)), top);
        assert_eq!(
            Big::pow2(128).to_string(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(
            Big::parse("-340282366920938463463374607431768211456"),
            Some(Big::pow2(128).neg())
        );
        assert_eq!(Big::parse("12x"), None);
        assert_eq!(Big::parse("-"), None);
        assert_eq!(b(-5).smt(), "(- 5)");
        assert_eq!(b(0).neg().to_string(), "0");
        assert_eq!(b(7).sub(&b(9)), b(-2));
        assert_eq!(b(-7).add(&b(9)), b(2));
        assert_eq!(b(-7).add(&b(-9)), b(-16));
        assert_eq!(b(-3).mul(&b(4)), b(-12));
        assert!(b(-3) < b(2) && b(-3) > b(-4) && b(5) > b(4) && Big::pow2(64) > b(i128::MAX >> 64));
        assert_eq!(b(i128::MIN).to_i128(), Some(i128::MIN));
        assert_eq!(b(i128::MIN).sub(&b(1)).to_i128(), None);
        assert_eq!(Big::from_u128(u128::MAX).to_i128(), None);
        assert_eq!(b(-1).to_u128(), None);
        assert_eq!(square.magnitude_u128(), None);
    }

    /// `div` and `mod` as SMT-LIB defines them: the remainder is never negative.
    #[test]
    fn test_div_and_mod_keep_the_remainder_non_negative() {
        for (m, n, q, r) in [
            (7, 2, 3, 1),
            (-7, 2, -4, 1),
            (7, -2, -3, 1),
            (-7, -2, 4, 1),
            (-8, 2, -4, 0),
            (8, -2, -4, 0),
            (-8, -2, 4, 0),
            (0, 5, 0, 0),
            (1, 7, 0, 1),
        ] {
            assert_eq!(b(m).div_mod(&b(n)), Some((b(q), b(r))), "{} div {}", m, n);
        }
        assert_eq!(b(3).div_mod(&b(0)), None);
        let big = Big::pow2(200).add(&b(5));
        let (q, r) = big.div_mod(&Big::pow2(100)).unwrap();
        assert_eq!((q, r), (Big::pow2(100), b(5)));
    }

    #[test]
    fn test_two_complement_bits_of_negative_values() {
        assert_eq!(b(-1).mod_pow2(8), b(255));
        assert_eq!(b(-256).mod_pow2(8), b(0));
        assert_eq!(b(-129).mod_pow2(8), b(127));
        assert_eq!(b(300).mod_pow2(8), b(44));
        assert_eq!(b(-1).mod_pow2(128), Big::from_u128(u128::MAX));
        assert_eq!(b(-1).mod_pow2(64), Big::from_u128(u128::from(u64::MAX)));
        assert_eq!(Big::pow2(130).add(&b(3)).mod_pow2(128), b(3));
    }
}

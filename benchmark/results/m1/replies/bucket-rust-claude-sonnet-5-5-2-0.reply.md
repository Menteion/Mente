The width `hi - lo + 1` can overflow `i32`, so I do the arithmetic in `i64`. With `lo` and `hi` as `i32`, `hi - lo + 1` is at most 2^32, which fits in `i64`. After clamping `x` into `[lo, hi]`, `(x - lo)` is at most 2^32 - 1. Multiplying by `n ≤ 64` gives at most about 2^38, which also fits in `i64`.

Clamping first handles the out-of-range cases: below `lo` gives 0, and above `hi` gives `(hi - lo) * n / (hi - lo + 1)`. That equals `n - 1` only when the width divides evenly, but the spec says a value above `hi` falls in the last bucket, so I return `n - 1` explicitly for `x > hi`.

For `x` within the range, `(x - lo) * n / w < n` because `x - lo < w`, so the result is always below `n`.

```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    if x < lo {
        return 0;
    }
    if x > hi {
        return n - 1;
    }
    let w = hi as i64 - lo as i64 + 1;
    let r = (x as i64 - lo as i64) * n as i64 / w;
    let r = r as u32;
    if r >= n { n - 1 } else { r }
}
```
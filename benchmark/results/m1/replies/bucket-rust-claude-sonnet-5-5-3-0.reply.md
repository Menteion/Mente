Widening to i128 keeps every intermediate exact, so overflow checks cannot fire. With lo and hi in the i32 range, `hi - lo + 1` is at most 2^32, and `(x - lo) * n` is far below the i128 limit.

Clamping first handles the out-of-range cases: if x < lo, use x = lo, which gives bucket 0. If x > hi, use x = hi, which gives (w - 1) * n / w. Since n ≤ w is not guaranteed, this can be below n but is still at most n - 1. The value (w - 1) * n / w is less than n, so it is the last bucket only when it equals n - 1. For w < n, some buckets are empty and the formula may not give n - 1 for hi. The spec says a value above hi falls in the last bucket, so I return n - 1 explicitly for x > hi. For x == hi the formula gives (w - 1) * n / w, which is below n. The spec defines the result by the formula for in-range values, so I keep it there.

For x > hi, I return n - 1. For x < lo, I return 0. Otherwise I apply the formula, which is always below n because x - lo ≤ w - 1.

```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    if x < lo {
        return 0;
    }
    if x > hi {
        return n - 1;
    }
    let w = hi as i128 - lo as i128 + 1;
    let d = x as i128 - lo as i128;
    let r = d * (n as i128) / w;
    r as u32
}
```
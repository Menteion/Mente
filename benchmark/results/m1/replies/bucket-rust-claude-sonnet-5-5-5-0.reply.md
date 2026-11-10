Overflow is the main concern: `hi - lo + 1` can exceed `i32`, so I compute in `i64` (or `i128` to be safe). The width is at most 2^32, `x - lo` is at most about 2^32, and `n` is at most 64, so the product is about 2^38, which fits in `i64`.

The steps:
1. If `x < lo`, return 0.
2. If `x > hi`, return `n - 1`.
3. Otherwise compute `(x - lo) * n / (hi - lo + 1)` in `i64`.

For `x` in `[lo, hi]`, `x - lo` is at most `hi - lo`, which is less than the width. So `(x - lo) * n < width * n`, and the quotient is less than `n`. I still clamp with `min(n - 1)` for safety.

```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    if x < lo {
        return 0;
    }
    if x > hi {
        return n - 1;
    }
    let width = hi as i64 - lo as i64 + 1;
    let off = x as i64 - lo as i64;
    let r = off * (n as i64) / width;
    (r as u32).min(n - 1)
}
```
The main concern is overflow: `hi - lo + 1` can exceed `i32` range, so all arithmetic is done in `i128`/`i64`.

The width is `hi - lo + 1`, which is at most 2^32, so it fits in `i64`. Clamping handles `x` outside the range. For `x` in `[lo, hi]`, `(x - lo) * n` is at most about 2^32 * 64 = 2^38, which also fits in `i64`.

If `x < lo`, the result is 0. If `x > hi`, the result is `n - 1`. Otherwise the formula gives `(x - lo) * n / width`. Since `x - lo <= width - 1`, we have `(x - lo) * n < width * n`, so the result is below `n`. The clamping for `x > hi` therefore matches the formula's maximum range, and the result stays under `n`.

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
    let r = off * n as i64 / width;
    let r = r as u32;
    if r >= n { n - 1 } else { r }
}
```
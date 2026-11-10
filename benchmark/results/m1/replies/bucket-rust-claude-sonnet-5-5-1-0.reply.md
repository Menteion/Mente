The computation is done in i128 so nothing overflows, even for extreme `lo` and `hi`.

First clamp `x` into `[lo, hi]`. Then `(x - lo) * n / (hi - lo + 1)`: the width is at most 2^32, `x - lo` is at most 2^32 - 1, and `n` is at most 64, so the product fits easily in i128.

The result is below `n`. With `x` clamped, `x - lo <= hi - lo < width`, so `(x - lo) * n / width < n`.

```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    let x = x.clamp(lo, hi) as i128;
    let lo = lo as i128;
    let hi = hi as i128;
    let width = hi - lo + 1;
    let r = (x - lo) * (n as i128) / width;
    r as u32
}
```
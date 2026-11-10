Widen to `i128`, which holds the exact sum of eight `i64` values with no overflow. Rust's integer division already truncates toward zero, so dividing the sum by 8 gives the required rounding. The mean of eight `i64` values always lies within the `i64` range, so the final cast back is safe.

```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
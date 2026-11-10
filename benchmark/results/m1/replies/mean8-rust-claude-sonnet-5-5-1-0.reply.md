Widen the arithmetic to `i128`: eight `i64` values sum to at most 8·2^63 = 2^66 in magnitude, which fits easily. Rust's `/` on integers already truncates toward zero, so dividing the `i128` sum by 8 gives the required rounding. The mean always lies within the `i64` range, so the final cast back is safe.

```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
Widen to `i128` so the sum can't overflow, then divide by 8. Rust's integer division truncates toward zero, which is the required rounding. The mean of eight `i64` values always lies within the `i64` range, so the final cast back is safe.

```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
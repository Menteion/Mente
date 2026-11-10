Widen to `i128`: the sum of eight `i64` values always fits, and Rust's integer division already truncates toward zero. The mean of eight `i64` values also lies within the `i64` range, so the final cast is safe.

```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
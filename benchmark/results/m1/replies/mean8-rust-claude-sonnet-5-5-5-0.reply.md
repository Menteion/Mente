Use i128 for the sum: eight i64 values sum to at most 8·2^63 = 2^66 in magnitude, which fits easily. Integer division in Rust truncates toward zero, which is the rounding required. The mean of eight i64 values always lies within the i64 range, so the cast back is safe.

```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    // The sum of eight i64 values fits in i128, and i128 division truncates toward zero.
    let sum: i128 = a.iter().map(|&x| x as i128).sum();
    (sum / 8) as i64
}
```
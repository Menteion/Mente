```rust
pub fn mean8(a: [i64; 8]) -> i64 {
    let s: i128 = a.iter().map(|&x| x as i128).sum();
    (s / 8) as i64
}
```
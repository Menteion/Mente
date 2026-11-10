```rust
/// The result is in `0..=16`.
pub fn lower_bound(a: [i32; 16], x: i32) -> usize {
    a.iter().filter(|&&v| v < x).count()
}
```
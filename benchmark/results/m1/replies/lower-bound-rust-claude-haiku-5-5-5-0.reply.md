```rust
/// The result is in `0..=16`.
pub fn lower_bound(a: [i32; 16], x: i32) -> usize {
    let mut i: usize = 0;
    while i < 16 && a[i] < x {
        i += 1;
    }
    i
}
```
```rust
/// The result is in `0..8`.
pub fn index_of_max(a: [i32; 8]) -> usize {
    let mut best = 0usize;
    let mut i = 1usize;
    while i < 8 {
        if a[i] > a[best] {
            best = i;
        }
        i += 1;
    }
    best
}
```
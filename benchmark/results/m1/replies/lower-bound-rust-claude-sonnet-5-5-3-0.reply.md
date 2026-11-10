```rust
/// The result is in `0..=16`.
pub fn lower_bound(a: [i32; 16], x: i32) -> usize {
    let mut lo: usize = 0;
    let mut hi: usize = 16;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if a[mid] >= x {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}
```
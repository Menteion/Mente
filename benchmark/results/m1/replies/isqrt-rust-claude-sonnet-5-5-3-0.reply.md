```rust
pub fn isqrt(n: u64) -> u64 {
    let mut lo: u64 = 0;
    let mut hi: u64 = u32::MAX as u64;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if mid * mid <= n {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}
```
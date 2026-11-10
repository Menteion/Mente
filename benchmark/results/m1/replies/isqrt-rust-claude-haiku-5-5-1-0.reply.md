```rust
pub fn isqrt(n: u64) -> u64 {
    let mut r = (n as f64).sqrt() as u64;
    while (r as u128) * (r as u128) > n as u128 {
        r -= 1;
    }
    while ((r + 1) as u128) * ((r + 1) as u128) <= n as u128 {
        r += 1;
    }
    r
}
```
```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    let lo = lo as i128;
    let hi = hi as i128;
    let x = (x as i128).clamp(lo, hi);
    let width = hi - lo + 1;
    let offset = x - lo;
    ((offset * n as i128) / width) as u32
}
```
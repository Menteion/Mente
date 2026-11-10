```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    let x = x.clamp(lo, hi);
    let num = (x as i128 - lo as i128) * n as i128;
    let den = hi as i128 - lo as i128 + 1;
    (num / den) as u32
}
```
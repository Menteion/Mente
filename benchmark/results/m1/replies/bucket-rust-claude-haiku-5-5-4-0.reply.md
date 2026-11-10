```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    let x = (x as i128).clamp(lo as i128, hi as i128);
    let off = x - lo as i128;
    let width = hi as i128 - lo as i128 + 1;
    ((off * n as i128) / width) as u32
}
```
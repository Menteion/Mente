```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    // Use i128 so that `hi - lo + 1` and the product with `n` cannot overflow.
    let lo128 = lo as i128;
    let hi128 = hi as i128;
    let xc = (x as i128).clamp(lo128, hi128);
    let width = hi128 - lo128 + 1;
    let r = (xc - lo128) * (n as i128) / width;
    r as u32
}
```
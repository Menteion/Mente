```rust
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    // Clamp the value into [lo, hi] so out-of-range values land in the end buckets.
    let x = x.clamp(lo, hi);
    // Use 128-bit arithmetic so that the widths and products cannot overflow.
    let offset = x as i128 - lo as i128;
    let width = hi as i128 - lo as i128 + 1;
    let result = offset * n as i128 / width;
    result as u32
}
```
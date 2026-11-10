```rust
pub fn reverse_bits(x: u8) -> u8 {
    let mut v = x;
    let mut r: u8 = 0;
    for _ in 0..8 {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}
```
```rust
pub fn to_byte(x: u32) -> u8 {
    // A u8 can't hold every u32 value, so this truncates to the low 8 bits.
    // It never panics, but values above 255 lose their high bits.
    x as u8
}
```
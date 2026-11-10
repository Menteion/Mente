```rust
pub fn to_byte(x: u32) -> u8 {
    // A lossless conversion is impossible for x > 255, so this truncates to the low 8 bits. It never panics.
    x as u8
}
```
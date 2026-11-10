```rust
pub fn to_byte(x: u32) -> u8 {
    // Not lossless for x > 255: a u8 can't hold those values, so this keeps only the low 8 bits (never panics).
    x as u8
}
```
```rust
pub fn to_byte(x: u32) -> u8 {
    // A u8 cannot represent every u32, so values above 255 are truncated to their low 8 bits.
    // `as` never panics, even with overflow checks on.
    x as u8
}
```
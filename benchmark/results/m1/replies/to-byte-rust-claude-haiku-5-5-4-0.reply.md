```rust
/// Returns the low 8 bits of `x`. Values above 255 are truncated, since a `u8`
/// cannot hold them. This never panics.
pub fn to_byte(x: u32) -> u8 {
    x as u8
}
```
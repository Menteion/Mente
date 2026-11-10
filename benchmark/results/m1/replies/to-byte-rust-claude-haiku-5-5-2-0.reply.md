```rust
/// Returns `x` as a `u8` when it fits (`x <= 255`), and is exact in that case.
/// A `u32` above 255 cannot be represented in a `u8`, so those inputs saturate to 255.
/// This never panics.
pub fn to_byte(x: u32) -> u8 {
    u8::try_from(x).unwrap_or(u8::MAX)
}
```
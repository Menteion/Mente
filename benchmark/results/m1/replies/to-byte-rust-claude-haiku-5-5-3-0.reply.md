The title promises a lossless conversion, but a `u32` above 255 has no `u8` representation, so no function can return `x` itself for every input. The function below keeps the low 8 bits for those values. It never panics.

```rust
/// Converts `x` to a `u8`. Values above 255 cannot be represented exactly,
/// so only the low 8 bits are kept (truncation). Never panics.
pub fn to_byte(x: u32) -> u8 {
    x as u8
}
```
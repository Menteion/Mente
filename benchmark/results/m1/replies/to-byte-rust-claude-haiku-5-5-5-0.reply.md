```rust
pub fn to_byte(x: u32) -> u8 {
    x as u8
}
```

No `u8` can hold every `u32`, so this can't return `x` unchanged for values above 255. The function above keeps the low 8 bits and never panics. It is exact for `x <= 255`, and for larger values it truncates. If you'd rather clamp to 255, use `u8::try_from(x).unwrap_or(u8::MAX)`.
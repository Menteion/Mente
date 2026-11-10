```rust
pub fn to_byte(x: u32) -> u8 {
    x as u8
}
```

No `u8` can hold values above 255, so this can't be lossless for every `u32`. The `as` cast keeps the low 8 bits and never panics, even in debug builds. If you'd rather clamp large values to 255, use `x.min(255) as u8` instead.
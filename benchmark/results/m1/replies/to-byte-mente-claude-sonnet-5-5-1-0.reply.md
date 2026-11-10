```mente
pub fn to_byte(x: u32) -> u8 {
  x & 255
}
```

No function with this signature can return `x` itself for every `u32` input, since values above 255 do not fit in a `u8`. This version never traps, but it returns `x` only when `x <= 255`. Otherwise it returns the low 8 bits of `x`. To return `x` exactly, the parameter type would have to be `u8` or `u32[0..256]`.
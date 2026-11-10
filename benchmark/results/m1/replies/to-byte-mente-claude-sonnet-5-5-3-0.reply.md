```mente
/// No u32 -> u8 conversion can both return `x` itself and never trap,
/// since values above 255 do not fit in a u8.
/// This version never traps, but it keeps only the low 8 bits (x mod 256).
pub fn to_byte(x: u32) -> u8 = x % 256
```

Returning `x` itself for every `u32` is impossible, so this does not meet the spec as written. It is exact only when `x <= 255`. For larger inputs it wraps instead of trapping.
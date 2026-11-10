```mente
pub fn to_byte(x: u32) -> u8
  requires x <= 255
{
  x
}
```

No function can meet both requirements. A `u32` can hold values above 255, and a `u8` cannot, so returning `x` itself for every input is impossible without trapping. I kept `x` unchanged and made the limit explicit with `requires x <= 255`. Every call must therefore prove the input fits, and for valid inputs it never traps. Masking with `x & 255` would be total, but it would return a different value for inputs above 255.
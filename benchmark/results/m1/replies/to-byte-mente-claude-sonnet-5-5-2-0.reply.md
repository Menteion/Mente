```mente
/// A u32 holds values a u8 cannot, so no total lossless conversion exists.
/// This is exact for every input in 0..=255, which `requires` states.
pub fn to_byte(x: u32) -> u8
  requires x <= 255
{
  x
}
```
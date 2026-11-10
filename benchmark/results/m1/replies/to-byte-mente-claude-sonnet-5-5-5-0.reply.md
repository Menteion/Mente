```mente
/// Not lossless: u32 values above 255 do not fit in u8, so this keeps the low 8 bits.
pub fn to_byte(x: u32) -> u8 {
  x & 255
}
```
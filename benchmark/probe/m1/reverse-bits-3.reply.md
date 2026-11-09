```mente
pub fn reverse_bits(x: u8) -> u8 {
  var r: u8 = 0
  var v: u8 = x
  var i: u8 = 0
  while i < 8 decreases 8 - i {
    r = (r << 1) | (v & 1)
    v = v >> 1
    i += 1
  }
  r
}
```
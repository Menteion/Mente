```mente
pub fn reverse_bits(x: u8) -> u8 {
  var v: u8 = x
  var r: u8 = 0
  var i: u32 = 0
  while i < 8 {
    r = (r << 1) | (v & 1)
    v = v >> 1
    i += 1
  }
  r
}
```
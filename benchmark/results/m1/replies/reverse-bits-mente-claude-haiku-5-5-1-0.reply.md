```mente
pub fn reverse_bits(x: u8) -> u8 {
  var v = x
  var r: u8 = 0
  var i: u8 = 0
  while i < 8 {
    r |= (v & 1) << (7 - i)
    v >>= 1
    i += 1
  }
  r
}
```
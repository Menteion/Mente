```mente
pub fn reverse_bits(x: u8) -> u8 {
  var v = x
  var result: u8 = 0
  var i: u8 = 0
  while i < 8 {
    result = result * 2 + (v & 1)
    v = v >> 1
    i += 1
  }
  result
}
```
```mente
pub fn reverse_bits(x: u8) -> u8 {
  var v: u8 = x
  var result: u8 = 0
  var i: u32 = 0
  while i < 8 {
    result = result * 2 + v % 2
    v = v / 2
    i += 1
  }
  result
}
```
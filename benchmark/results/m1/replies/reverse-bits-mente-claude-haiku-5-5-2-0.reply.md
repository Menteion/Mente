```mente
pub fn reverse_bits(x: u8) -> u8 {
  var r: u128 = 0
  var v: u128 = x
  var i: u32 = 0
  while i < 8 {
    r = r * 2 + v % 2
    v = v / 2
    i += 1
  }
  r
}
```
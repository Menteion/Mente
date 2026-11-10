```mente
pub fn reverse_bits(x: u8) -> u8 {
  var rest: u8 = x
  var acc: u8 = 0
  var i: u32 = 0
  while i < 8 {
    acc = (acc << 1) | (rest & 1)
    rest = rest >> 1
    i += 1
  }
  acc
}
```
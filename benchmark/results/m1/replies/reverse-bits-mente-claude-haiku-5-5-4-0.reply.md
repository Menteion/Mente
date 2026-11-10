```mente
pub fn reverse_bits(x: u8) -> u8 {
  var v: u8 = x
  var result: u8 = 0
  var n: u8 = 0
  while n < 8 {
    val bit: u8 = v % 2
    result = result * 2 + bit
    v = v / 2
    n += 1
  }
  result
}
```
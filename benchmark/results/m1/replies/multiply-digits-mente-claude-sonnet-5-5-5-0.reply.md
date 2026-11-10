```mente
pub fn multiply_digits(a: [6]u8, b: [6]u8) -> [12]u8 {
  var result: [12]u8 = [0; 12]
  var i: usize = 0
  while i < 6 {
    var carry: u32 = 0
    var j: usize = 0
    while j < 6 {
      val cur: u32 = result[i + j] + a[i] * b[j] + carry
      result[i + j] = cur % 10
      carry = cur / 10
      j += 1
    }
    result[i + 6] = carry
    i += 1
  }
  result
}
```
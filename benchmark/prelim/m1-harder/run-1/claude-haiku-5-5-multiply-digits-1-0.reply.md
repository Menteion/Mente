```mente
pub fn multiply_digits(a: [6]u8, b: [6]u8) -> [12]u8 {
  var r: [12]u8 = [0; 12]
  var i: usize = 0
  while i < 6 {
    var carry: u32 = 0
    var j: usize = 0
    while j < 6 {
      val p: u32 = a[i] * b[j]
      val old: u32 = r[i + j]
      val cur: u32 = old + p + carry
      r[i + j] = cur % 10
      carry = cur / 10
      j += 1
    }
    r[i + 6] = carry
    i += 1
  }
  r
}
```
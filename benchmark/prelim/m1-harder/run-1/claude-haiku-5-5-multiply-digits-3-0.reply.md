```mente
pub fn multiply_digits(a: [6]u8, b: [6]u8) -> [12]u8 {
  var acc: [12]u32 = [0; 12]
  var i: usize = 0
  while i < 6 {
    var j: usize = 0
    while j < 6 {
      val p: u32 = a[i] * b[j]
      acc[i + j] = acc[i + j] + p
      j += 1
    }
    i += 1
  }
  var carry: u32 = 0
  var k: usize = 0
  while k < 12 {
    val total: u32 = acc[k] + carry
    acc[k] = total % 10
    carry = total / 10
    k += 1
  }
  var out: [12]u8 = [0; 12]
  var m: usize = 0
  while m < 12 {
    out[m] = acc[m]
    m += 1
  }
  out
}
```
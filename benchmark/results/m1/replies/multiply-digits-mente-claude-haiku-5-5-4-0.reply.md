```mente
pub fn multiply_digits(a: [6]u8, b: [6]u8) -> [12]u8 {
  var acc: [12]u32 = [0; 12]
  var i: usize = 0
  while i < 6 {
    var j: usize = 0
    while j < 6 {
      acc[i + j] = acc[i + j] + a[i] * b[j]
      j += 1
    }
    i += 1
  }
  var k: usize = 0
  while k < 11 {
    acc[k + 1] = acc[k + 1] + acc[k] / 10
    acc[k] = acc[k] % 10
    k += 1
  }
  var r: [12]u8 = [0; 12]
  var m: usize = 0
  while m < 12 {
    r[m] = acc[m] % 10
    m += 1
  }
  r
}
```
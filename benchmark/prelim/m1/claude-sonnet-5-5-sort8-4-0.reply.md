```mente
pub fn sort8(a: [8]i32) -> [8]i32 {
  var b: [8]i32 = a
  var i: usize = 0
  while i < 8 {
    var j: usize = 0
    while j < 7 - i {
      if b[j] > b[j + 1] {
        val t = b[j]
        b[j] = b[j + 1]
        b[j + 1] = t
      }
      j += 1
    }
    i += 1
  }
  b
}
```
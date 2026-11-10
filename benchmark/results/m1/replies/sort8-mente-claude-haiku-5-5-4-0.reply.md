```mente
pub fn sort8(a: [8]i32) -> [8]i32 {
  var b: [8]i32 = a
  var i: usize = 1
  while i < 8 {
    val key = b[i]
    var j: usize = i
    while j > 0 {
      if b[j-1] <= key {
        break
      }
      b[j] = b[j-1]
      j -= 1
    }
    b[j] = key
    i += 1
  }
  b
}
```
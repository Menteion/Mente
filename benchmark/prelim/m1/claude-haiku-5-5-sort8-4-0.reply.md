```mente
pub fn sort8(a: [8]i32) -> [8]i32 {
  var v: [8]i32 = a
  var i: usize = 1
  while i < 8 {
    var j: usize = i
    while j > 0 {
      guard v[j - 1] > v[j] else {
        break
      }
      val t = v[j]
      v[j] = v[j - 1]
      v[j - 1] = t
      j -= 1
    }
    i += 1
  }
  v
}
```
```mente
pub fn sort8(a: [8]i32) -> [8]i32 {
  var v = a
  var i: usize = 0
  while i < 7 {
    var j: usize = 0
    while j < 7 {
      if v[j] > v[j + 1] {
        val t = v[j]
        v[j] = v[j + 1]
        v[j + 1] = t
      }
      j += 1
    }
    i += 1
  }
  v
}
```
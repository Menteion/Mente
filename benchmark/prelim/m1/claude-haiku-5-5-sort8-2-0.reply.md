```mente
pub fn sort8(a: [8]i32) -> [8]i32 {
  var v: [8]i32 = a
  var i: usize = 1
  while i < 8 {
    val key = v[i]
    var j: usize = i
    while j > 0 {
      if v[j - 1] <= key {
        break
      }
      v[j] = v[j - 1]
      j -= 1
    }
    v[j] = key
    i += 1
  }
  v
}
```
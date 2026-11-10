```mente
pub fn reverse_in_place(mut a: [10]i32) {
  var i: usize = 0
  var j: usize = 9
  while i < j {
    val t = a[i]
    a[i] = a[j]
    a[j] = t
    i += 1
    j -= 1
  }
}
```
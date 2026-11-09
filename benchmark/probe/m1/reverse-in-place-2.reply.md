```mente
pub fn reverse_in_place(mut a: [10]i32) {
  var i: usize = 0
  while i < 5 invariant i <= 5 decreases 5 - i {
    val j: usize = 9 - i
    val t = a[i]
    a[i] = a[j]
    a[j] = t
    i += 1
  }
}
```
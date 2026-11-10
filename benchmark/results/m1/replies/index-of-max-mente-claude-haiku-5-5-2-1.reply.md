```mente
pub fn index_of_max(a: [8]i32) -> usize[0..8] {
  var best: usize[0..8] = 0
  var largest: i32 = a[0]
  var i: usize = 1
  while i < 8 {
    if a[i] > largest {
      largest = a[i]
      best = i
    }
    i += 1
  }
  best
}
```
```mente
pub fn index_of_max(a: [8]i32) -> usize[0..8] {
  var best: usize[0..8] = 0
  var i: usize = 1
  while i < 8 invariant 1 <= i and i <= 8 decreases 8 - i {
    if a[i] > a[best] {
      best = i
    }
    i += 1
  }
  best
}
```
The error came from `i`. Its type `usize[0..8]` holds only 0 to 7, so `i += 1` produces 8 on the last pass, which is outside that type. Widening `i` to `usize[0..=8]` lets it reach 8, and the loop condition `i < 8` still guarantees each `a[i]` access and the assignment `best = i` stay in range.

```mente
pub fn index_of_max(a: [8]i32) -> usize[0..8] {
  var best: usize[0..8] = 0
  var i: usize[0..=8] = 1
  while i < 8 {
    if a[i] > a[best] {
      best = i
    }
    i += 1
  }
  best
}
```
The error is on `i`. Its refined type `usize[0..8]` cannot hold the value 8 that `i += 1` produces after the last iteration. Declaring `i` as a plain `usize` fixes this. The loop condition `i < 8` still lets the compiler prove that `best = i` and `a[i]` stay in range.

```mente
pub fn index_of_max(a: [8]i32) -> usize[0..8] {
  var best: usize[0..8] = 0
  var i: usize = 1
  while i < 8 {
    if a[i] > a[best] {
      best = i
    }
    i += 1
  }
  best
}
```
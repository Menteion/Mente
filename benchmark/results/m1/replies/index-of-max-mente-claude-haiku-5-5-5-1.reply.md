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

The counter `i` is now a plain `usize`, so `i += 1` can reach 8 without a refinement error. The loop condition `i < 8` tells the compiler that `i` is in range when `best = i` runs, so `best` keeps its `usize[0..8]` type.
```mente
pub fn max_subarray_sum(a: [12]i32) -> i64 {
  var cur: i64 = a[0]
  var best: i64 = a[0]
  var i: usize = 1
  while i < 12 {
    if cur > 0 {
      cur += a[i]
    } else {
      cur = a[i]
    }
    if cur > best {
      best = cur
    }
    i += 1
  }
  best
}
```
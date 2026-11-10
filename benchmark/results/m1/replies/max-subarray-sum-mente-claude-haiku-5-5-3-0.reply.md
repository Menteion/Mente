```mente
pub fn max_subarray_sum(a: [12]i32) -> i64 {
  var best: i64 = a[0]
  var cur: i64 = a[0]
  var i: usize = 1
  while i < 12 {
    val v: i64 = a[i]
    if cur > 0 {
      cur += v
    } else {
      cur = v
    }
    if cur > best {
      best = cur
    }
    i += 1
  }
  best
}
```
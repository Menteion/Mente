```mente
pub fn max_subarray_sum(a: [12]i32) -> i64 {
  var best: i64 = a[0]
  var cur: i64 = a[0]
  var i: usize = 1
  while i < 12 {
    val x = a[i]
    if cur > 0 {
      cur = cur + x
    } else {
      cur = x
    }
    if cur > best {
      best = cur
    }
    i += 1
  }
  best
}
```
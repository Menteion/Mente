```mente
pub fn max_subarray_sum(a: [12]i32) -> i64 {
  var best: i64 = a[0]
  var cur: i64 = a[0]
  var i: usize = 1
  while i < a.len {
    val x: i64 = a[i]
    val extended: i64 = cur + x
    if extended > x {
      cur = extended
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
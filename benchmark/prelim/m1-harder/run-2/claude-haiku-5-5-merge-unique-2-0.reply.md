```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
  var started: bool = false
  while i < 8 and j < 8 {
    val x = a[i]
    val y = b[j]
    val m = if x <= y { x } else { y }
    if x <= y {
      i += 1
    }
    if x > y {
      j += 1
    }
    if not started or m != last {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = m
      k += 1
      last = m
      started = true
    }
  }
  while i < 8 {
    val p = a[i]
    if not started or p != last {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = p
      k += 1
      last = p
      started = true
    }
    i += 1
  }
  while j < 8 {
    val q = b[j]
    if not started or q != last {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = q
      k += 1
      last = q
      started = true
    }
    j += 1
  }
  guard k <= 16 else {
    unreachable("count never exceeds 16")
  }
  k
}
```
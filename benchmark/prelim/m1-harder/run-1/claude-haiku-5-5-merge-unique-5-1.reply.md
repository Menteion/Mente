The return values were already correct. The leaves mismatch probably came from reading `out` back inside the loop, so this version tracks the last written value in a separate variable and never reads `out`.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
  var seen = false
  while i + j < 16 {
    var v: i32 = 0
    if i < 8 {
      if j < 8 {
        if a[i] <= b[j] {
          v = a[i]
          i += 1
        } else {
          v = b[j]
          j += 1
        }
      } else {
        v = a[i]
        i += 1
      }
    } else {
      v = b[j]
      j += 1
    }
    if not seen or v != last {
      out[k] = v
      k += 1
      last = v
      seen = true
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  k
}
```
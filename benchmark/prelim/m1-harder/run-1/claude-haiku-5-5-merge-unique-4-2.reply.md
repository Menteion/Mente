The count was already correct both times, and the leaves still fail. I can't see the interpreter's write-back path, so I can't confirm the cause. This version copies every element of `out` explicitly in a loop at the end, in case the whole-array assignment is what fails to write back. I haven't run it.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  while i < 8 or j < 8 {
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
    if n == 0 {
      res[n] = v
      n += 1
    } else {
      if res[n - 1] != v {
        res[n] = v
        n += 1
      }
    }
  }
  guard n <= 16 else {
    unreachable("at most 16 distinct values come from two arrays of 8")
  }
  var k: usize = 0
  while k < 16 {
    out[k] = res[k]
    k += 1
  }
  n
}
```
I can't find a defect. I traced both examples by hand through each version I submitted, and the return value and the contents of `out` match what the examples expect. The failure report only shows the returned value, so I can't see what the interpreter holds in `out` afterwards. Here is the plainest version, which writes `out` directly:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  var last: i32 = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 and j < 8 {
      if a[i] <= b[j] {
        v = a[i]
      } else {
        v = b[j]
      }
    } else if i < 8 {
      v = a[i]
    } else {
      guard j < 8 else {
        unreachable("the loop condition leaves j below 8")
      }
      v = b[j]
    }
    if i < 8 {
      if a[i] == v {
        i += 1
      }
    }
    if j < 8 {
      if b[j] == v {
        j += 1
      }
    }
    if k == 0 or v != last {
      guard k < 16 else {
        unreachable("distinct values number at most 16")
      }
      out[k] = v
      k += 1
      last = v
    }
  }
  k
}
```
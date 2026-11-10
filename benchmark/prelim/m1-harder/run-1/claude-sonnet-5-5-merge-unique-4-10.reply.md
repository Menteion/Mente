I can't produce a different result. The only thing the report shows is the returned value, and it is correct in both examples (12 and 8). I have traced the `out` contents by hand, and they match the expected `leaves` arrays. I have also tried writing `out` four ways: directly, through a copy assigned back as a whole, through an element-by-element copy-back, and with an explicit `return`. All of them gave the same report, so I believe the reported mismatch is not in this function's logic. Here is the same function again:

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
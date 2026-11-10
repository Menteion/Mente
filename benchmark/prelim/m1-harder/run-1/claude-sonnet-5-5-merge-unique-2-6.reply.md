I can't fix this by changing the function. The code is the same as my last reply, because I still see nothing wrong with it.

- **Return value:** the interpreter's output shows 12 and 8, matching the expected values.
- **Contents of `out`:** tracing both examples by hand, I get exactly the expected `leaves` arrays. The merge writes each distinct value once, in increasing order, into the first `k` slots and never touches the rest.
- **What I tried:** writing directly into `out`, writing through a local buffer and copying back with `out = buf`, a post-pass copy loop, and several loop structures. All of them failed identically, and the report gives no actual `leaves` value to compare. That suggests the problem is in how the `leaves` check reads the `mut` argument, not in the algorithm.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
        }
      }
    } else {
      if j < 8 {
        v = b[j]
      }
    }
    if k < 16 {
      out[k] = v
      k += 1
    }
    while i < 8 {
      if a[i] != v {
        break
      }
      i += 1
    }
    while j < 8 {
      if b[j] != v {
        break
      }
      j += 1
    }
  }
  k
}
```

If the interpreter can print the actual `out` it sees after the call, I can use that to find the difference.
I'm not going to make another speculative edit. Each variation I've tried has produced the same report, so I have no evidence about what is wrong, and more guessing would only churn the code.

- **What I've established:** the interpreter reports the return values 12 and 8 matching the expected ones. By hand, the function leaves `out` exactly as the examples require: the distinct values in increasing order in the first `k` slots, and the rest untouched.
- **What I can't see:** the report never shows the `out` the interpreter actually holds, so I can't tell how it differs from the expected array.
- **What I've ruled out:** writing in place versus through a local copy, refined versus plain index variables, `break` loops versus flag loops, and tail expression versus `return`. None of these changed the report.

This is the same merge as before, with a plain `usize` index and an explicit invariant:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 invariant k <= 16 {
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

If the interpreter can print the `out` it holds after the call, or the failure can be reproduced with a smaller input, I can find the cause from that.
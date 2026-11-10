I'm going to stop resubmitting. Five attempts have produced the same report, and the return values match each time. I've traced both examples by hand, and the array contents come out exactly as expected. So the rewrites aren't the issue, and another one would be identical in substance.

The report doesn't show what the interpreter sees in `out` after the call, so I can't pin the failure to a specific line. The most likely cause is that the harness doesn't write a literal `mut` argument back. Two things would settle it:

1. Run the first example with the array in a `var`: `var o: [16]i32 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]`, then `merge_unique(a, b, mut o)`, then check `o`. If `o` has the expected leaves, the function is correct.
2. If `o` is wrong, send the array the interpreter returns and I'll find the difference.

For reference, this is the function I'm keeping:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var v: i32 = 0
  var fresh = true
  while i < 8 or j < 8 {
    if i < 8 and j < 8 {
      if a[i] <= b[j] {
        v = a[i]
        i += 1
      } else {
        v = b[j]
        j += 1
      }
    } else if i < 8 {
      v = a[i]
      i += 1
    } else {
      v = b[j]
      j += 1
    }
    fresh = true
    if k > 0 {
      if out[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      out[k] = v
      k += 1
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  k
}
```
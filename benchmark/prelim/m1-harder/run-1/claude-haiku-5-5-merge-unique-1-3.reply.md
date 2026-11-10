I've now sent essentially the same algorithm four times, and the interpreter reports the same failure each time. The return value is correct in both examples. Only the contents of `out` differ, and I can't see those from the report.

I traced both examples by hand against the expected leaves, and the algorithm produces exactly `[1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 15, 0, 0, 0, 0]` and `[1, ..., 8, -1, ...]`. So the logic appears correct, and the mismatch is probably in how the mutable array argument is handled. The examples pass a literal as the `mut` argument, and the card describes a `mut` parameter as receiving a place. A literal may not be a place, so its copy-back may not behave as the harness expects. I can't confirm that from the card.

Could you check two things on the interpreter side?
1. Whether `mut [ ... ]` with a literal is accepted as a place, or whether the harness needs the array in a `var`.
2. The actual leaves value the interpreter returns for the first example.

With that, I can tell whether the function needs to change. Until then, this is the same function as before:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
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
      if res[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      res[k] = v
      k += 1
    }
  }
  out = res
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  k
}
```
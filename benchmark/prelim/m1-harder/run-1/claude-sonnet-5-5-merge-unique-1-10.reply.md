I can't make progress on this one. The interpreter's report is the same for every version I have tried. It shows only the returned count (12 and 8), which matches, and not the final contents of `out`. I have hand-traced both examples through each version, including this one, and I get the expected count and the expected `out`. I can't see anything in the merge to change without guessing.

This is the same function as my last reply, unchanged:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  while i < 8 or j < 8 {
    var take_a: bool = false
    if i < 8 {
      take_a = true
      if j < 8 {
        take_a = a[i] <= b[j]
      }
    }
    var v: i32 = 0
    if take_a {
      guard i < 8 else {
        unreachable("take_a implies i < 8")
      }
      v = a[i]
      i += 1
    } else {
      guard j < 8 else {
        unreachable("without a, the loop condition leaves j < 8")
      }
      v = b[j]
      j += 1
    }
    var is_new: bool = true
    if n > 0 {
      guard n <= 16 else {
        unreachable("n never exceeds 16")
      }
      if out[n - 1] == v {
        is_new = false
      }
    }
    if is_new {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
    }
  }
  guard n <= 16 else {
    unreachable("n never exceeds 16")
  }
  return n
}
```

If you can show the actual `out` the interpreter holds after the call, I can compare it with my trace and find where they differ.
The return values are correct in both examples, and tracing the algorithm by hand gives the expected `leaves` contents. So the failure is in how the `mut` array is written back, not in the merge logic. I can't see the actual `leaves` values, so I can't confirm the cause. This version avoids element writes to the `mut` parameter altogether. It copies `out` into a local `var`, does all the element writes there, and assigns the whole array back once at the end.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 {
    val v: i32 = if i >= 8 { b[j] } else if j >= 8 { a[i] } else if a[i] <= b[j] { a[i] } else { b[j] }
    val from_a: bool = if i >= 8 { false } else { a[i] == v }
    val from_b: bool = if j >= 8 { false } else { b[j] == v }
    val fresh: bool = if k == 0 { true } else { res[k - 1] != v }
    res[k] = if fresh { v } else { res[k] }
    if fresh {
      k += 1
    }
    if from_a {
      i += 1
    }
    if from_b {
      j += 1
    }
  }
  out = res
  guard k <= 16 else {
    unreachable("the count never exceeds the length of out")
  }
  k
}
```
The return values were right, so the problem is in the writes to `out`. I've moved the write out of the nested `if` and its `guard`, and it now happens at the top level of the loop body. The loop runs at most 16 times, and `k` only grows on fresh values, so `k` stays below 16 whenever `out[k]` is accessed. The bound check is therefore not needed inside the loop.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 {
    val v: i32 = if i >= 8 { b[j] } else if j >= 8 { a[i] } else if a[i] <= b[j] { a[i] } else { b[j] }
    val from_a: bool = if i >= 8 { false } else { a[i] == v }
    val from_b: bool = if j >= 8 { false } else { b[j] == v }
    val fresh: bool = if k == 0 { true } else { out[k - 1] != v }
    out[k] = if fresh { v } else { out[k] }
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
  guard k <= 16 else {
    unreachable("the count never exceeds the length of out")
  }
  k
}
```
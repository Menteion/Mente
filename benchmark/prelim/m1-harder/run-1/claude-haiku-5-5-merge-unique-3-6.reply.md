I've now given four different versions, and the harness reports the same `leaves` mismatch for each. This version changes one thing: the final whole-array assignment `out = res` becomes an element-by-element copy loop. If the harness still fails, the cause is almost certainly not in this function.

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
  var t: usize = 0
  while t < 16 {
    out[t] = res[t]
    t += 1
  }
  guard k <= 16 else {
    unreachable("the count never exceeds the length of out")
  }
  k
}
```

I can't see from the harness output what `leaves` value the function produced, so I can't tell which slots differ. I've traced both examples by hand and they should produce the expected arrays. If this version also fails, the problem is most likely in how the reference interpreter handles writes to a `mut` array argument, not in the merge logic. A `mut` probe like `mut_probe` from my earlier reply, run separately, would confirm that.
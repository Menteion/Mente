My last reply put the probe in a `mente` block, so the harness ran that block and found no `merge_unique`. Here is the merge function alone, in one block:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 {
    guard k < 16 else {
      unreachable("at most 16 distinct values are written")
    }
    val v: i32 = if i < 8 {
      if j < 8 {
        if a[i] <= b[j] { a[i] } else { b[j] }
      } else {
        a[i]
      }
    } else {
      b[j]
    }
    val fresh = if k == 0 { true } else { out[k - 1] != v }
    if fresh {
      out[k] = v
      k += 1
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
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values are written")
  }
  k
}
```

The earlier leaves failures were a separate problem, and I haven't changed the algorithm. Tracing both examples by hand gives the expected leaves, so if the leaves still fail on this run, the output of the reference interpreter will be needed to find the cause.
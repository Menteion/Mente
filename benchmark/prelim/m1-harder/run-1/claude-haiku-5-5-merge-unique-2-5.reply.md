The return values are right in every run, so the merge logic seems sound. The leaves still fail, and I can't see the leaves the interpreter produced, so I can't prove the cause. One thing I can change is how often the function reads the `mut` array. This version tracks the last value written in a local `var last`, so it never reads `out` after writing to it, and it writes to `out` only at each new position:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
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
    val fresh = if k == 0 { true } else { last != v }
    if fresh {
      out[k] = v
      last = v
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

If the leaves still fail, the interpreter's output for the `mut` argument, the final leaves array, would show whether writes are reaching the caller at all.
I've sent this same function four times now, and the report hasn't changed. The return values match in both examples. The function writes exactly the expected leaves when I trace it by hand, so I don't think a fifth rewrite will help.

The only difference the report shows is in the array contents after the call, and that is the part the harness observes through a literal `mut` argument. Until that is resolved, the function below is the one I'd keep:

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

The check that would settle it is to run the first example with the array in a `var`, not a literal: `var o: [16]i32 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]` followed by `merge_unique(a, b, mut o)`, then inspect `o`. If `o` holds the expected leaves, the function is correct and the problem is in how the harness handles literal `mut` arguments. If it doesn't, send me what `o` contains and I'll look at the difference directly.
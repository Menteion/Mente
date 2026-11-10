The examples still fail on `leaves`, and I can't fix that from the function body. The return values are correct, and I traced the array result by hand on all three examples. Here is the function, unchanged:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found: bool = false
  var pivot: usize = 0
  var j: usize = 5
  while j > 0 and not found {
    if a[j - 1] < a[j] {
      found = true
      pivot = j - 1
    }
    j -= 1
  }
  if not found {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t1 = a[lo]
      a[lo] = a[hi]
      a[hi] = t1
      lo += 1
      hi -= 1
    }
    return false
  }
  var s: usize = 5
  while s > pivot and a[s] <= a[pivot] {
    s -= 1
  }
  val t2 = a[pivot]
  a[pivot] = a[s]
  a[s] = t2
  var lo2: usize = pivot + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val t3 = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = t3
    lo2 += 1
    hi2 -= 1
  }
  true
}
```

The `leaves` failure has persisted through every version, including the one that works on a local copy. That points away from the function and toward the harness or the call form, so more rewrites won't help.

To make progress, I need one of these from the reference interpreter:

1. The value it reports for `a` after the first example's call.
2. The result of the `set_first` probe from earlier: whether a `var` argument shows the write-back.

With either one, I can tell whether to change the function or the examples.
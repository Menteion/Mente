I have no new fix. The report has been identical for every version I've sent, and it still shows only the returned value (`true`, `false`, `true`, all correct). The array I leave in `a` is not shown, so I can't see how it differs from the expected one. Tracing the code by hand gives exactly the expected arrays for all three examples, so I won't change the algorithm without evidence that it is wrong. I'm resending the version I trust most:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize[0..6] = 5
  while i > 0 and a[i - 1] >= a[i] {
    i -= 1
  }
  var lo: usize[0..6] = 0
  var hi: usize[0..6] = 5
  if i > 0 {
    val p: usize[0..6] = i - 1
    var j: usize[0..6] = 5
    while j > i and a[j] <= a[p] {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
    a[j] = t
    lo = i
  }
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  i > 0
}
```

If you can run one example and show me the array the interpreter left in `a`, I can tell whether the fault is in my code or in how the `mut` result is checked.
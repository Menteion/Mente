```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 {
    if a[i - 1] < a[i] {
      break
    }
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while a[j] <= a[p] {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
    a[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
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

This is the same program as before, and I still can't see what makes it fail. By hand it gives the right return value and the right final array for all three examples. Four different structures (early return, no `break`, a local copy assigned back to `a`, and this one) have all produced the same report. Every report shows the right return value and says nothing about the array the call left behind. That suggests the mismatch is in how the checker reads the `mut` argument's final value, not in the algorithm. If you can show me the array the interpreter actually ended with, I can pin down the difference.
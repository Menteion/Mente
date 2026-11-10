I have no new information to act on. The output is identical to the last several rounds and reports only the return value, which is correct in all three examples. Hand traces of the three examples give the expected final arrays, and versions with different structure (local copy, early return, no invariants) all fail with this same message. So I don't think another rewrite will change the result. This is the version I trust:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while j > i and a[j] <= a[i - 1] invariant j <= 5 {
      j -= 1
    }
    val t = a[i - 1]
    a[i - 1] = a[j]
    a[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  i > 0
}
```

If the harness can print the array it sees after the call, I can pin down the difference from that.
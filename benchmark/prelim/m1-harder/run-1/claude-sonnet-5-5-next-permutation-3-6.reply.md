I can't find anything wrong with this function. I traced all three examples by hand and each gives the expected return value and final array. The reported return values match too, and the output never shows what the array was left holding. So I suspect the `leaves` check is failing in the interpreter or harness, not in the algorithm. I'm resending the straightforward version.

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

If the harness can show the array actually left after the call, I can use that to find what differs.
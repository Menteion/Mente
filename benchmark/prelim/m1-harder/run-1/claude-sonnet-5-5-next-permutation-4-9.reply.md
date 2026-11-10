I have no further change to offer. The function below is the straightforward version again. Hand traces of all three examples match the expected return values and final arrays, and eight differently structured versions have failed with the same output. That output shows only the return value, so I can't see how the final array differs, and I won't keep rewriting code I believe is correct. If you can show the array the interpreter produces, I can fix whatever is wrong from that.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while j > i and a[j] <= a[p] invariant j <= 5 {
      j -= 1
    }
    val tmp = a[p]
    a[p] = a[j]
    a[j] = tmp
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val t = a[lo]
    a[lo] = a[hi]
    a[hi] = t
    lo += 1
    hi -= 1
  }
  i > 0
}
```
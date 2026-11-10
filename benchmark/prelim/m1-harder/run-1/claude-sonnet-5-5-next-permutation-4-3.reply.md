This version drops the refined index types and uses plain `usize` counters, with loop invariants that keep every index in range. The three results so far were identical, so the cause may be how refined `var`s interact with the final array. It works directly on `a` again.

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
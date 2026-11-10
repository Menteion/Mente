```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i]
    invariant i <= 5
  {
    i -= 1
  }
  var lo: usize = 0
  var hi: usize = 5
  var found = false
  if i > 0 {
    found = true
    val p = i - 1
    var j: usize = 5
    while j > i and a[j] <= a[p]
      invariant j <= 5
    {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
    a[j] = t
    lo = i
  }
  while lo < hi
    invariant hi <= 5
  {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  found
}
```
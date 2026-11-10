```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  var found = false
  while i > 0 and not found {
    if a[i - 1] < a[i] {
      found = true
    } else {
      i -= 1
    }
  }
  if found {
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
  found
}
```
```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 invariant i <= 5 {
    if a[i - 1] < a[i] {
      break
    }
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while j > i invariant j <= 5 {
      if a[j] > a[i - 1] {
        break
      }
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
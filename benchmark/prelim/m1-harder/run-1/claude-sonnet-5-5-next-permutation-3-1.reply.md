```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize = 5
  while i > 0 and b[i - 1] >= b[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while j > i and b[j] <= b[p] invariant j <= 5 {
      j -= 1
    }
    val t = b[p]
    b[p] = b[j]
    b[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val u = b[lo]
    b[lo] = b[hi]
    b[hi] = u
    lo += 1
    hi -= 1
  }
  val found = i > 0
  a = b
  return found
}
```
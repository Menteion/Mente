```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize = 5
  var searching = true
  while i > 0 and searching {
    if b[i - 1] < b[i] {
      searching = false
    } else {
      i -= 1
    }
  }
  val has_next = i > 0
  if has_next {
    val pivot = b[i - 1]
    var j: usize = 5
    while j > i and b[j] <= pivot {
      j -= 1
    }
    val t = b[i - 1]
    b[i - 1] = b[j]
    b[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi {
    val u = b[lo]
    b[lo] = b[hi]
    b[hi] = u
    lo += 1
    hi -= 1
  }
  a = b
  return has_next
}
```
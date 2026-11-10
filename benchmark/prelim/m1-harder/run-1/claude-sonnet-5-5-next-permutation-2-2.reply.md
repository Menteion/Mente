```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize = 5
  var searching = true
  while searching {
    if i == 0 {
      searching = false
    } else if b[i - 1] < b[i] {
      searching = false
    } else {
      i -= 1
    }
  }
  val found = i > 0
  if found {
    val p = i - 1
    var j: usize = 5
    while b[j] <= b[p] {
      j -= 1
    }
    val t = b[p]
    b[p] = b[j]
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
  found
}
```
```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize[0..6] = 5
  while i > 0 and b[i - 1] >= b[i] {
    i -= 1
  }
  var lo: usize[0..6] = 0
  var hi: usize[0..6] = 5
  if i > 0 {
    val p: usize[0..6] = i - 1
    var j: usize[0..6] = 5
    while j > i and b[j] <= b[p] {
      j -= 1
    }
    val t = b[p]
    b[p] = b[j]
    b[j] = t
    lo = i
  }
  while lo < hi {
    val u = b[lo]
    b[lo] = b[hi]
    b[hi] = u
    lo += 1
    hi -= 1
  }
  a = b
  i > 0
}
```
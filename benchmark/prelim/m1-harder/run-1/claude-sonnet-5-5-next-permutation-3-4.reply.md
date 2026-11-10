```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = [0; 6]
  var k: usize = 0
  while k < 6 invariant k <= 6 {
    b[k] = a[k]
    k += 1
  }
  var i: usize = 5
  while i > 0 and b[i - 1] >= b[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while j > i and b[j] <= b[i - 1] invariant j <= 5 {
      j -= 1
    }
    val t = b[i - 1]
    b[i - 1] = b[j]
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
  var m: usize = 0
  while m < 6 invariant m <= 6 {
    a[m] = b[m]
    m += 1
  }
  i > 0
}
```
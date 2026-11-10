```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var r: [6]i32 = [0; 6]
  var k: usize = 0
  while k < 6 {
    r[k] = a[k]
    k += 1
  }
  var p: usize = 5
  var found = false
  while p > 0 and not found {
    if r[p - 1] < r[p] {
      found = true
    } else {
      p -= 1
    }
  }
  if found {
    var j: usize = 5
    while j > p and r[j] <= r[p - 1] {
      j -= 1
    }
    val t = r[p - 1]
    r[p - 1] = r[j]
    r[j] = t
  }
  var lo: usize = p
  var hi: usize = 5
  while lo < hi {
    val u = r[lo]
    r[lo] = r[hi]
    r[hi] = u
    lo += 1
    hi -= 1
  }
  a = r
  return found
}
```
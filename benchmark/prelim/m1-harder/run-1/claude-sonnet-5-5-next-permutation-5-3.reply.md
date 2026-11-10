```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 4
  while p > 0 and a[p] >= a[p + 1] {
    p -= 1
  }
  if a[p] >= a[p + 1] {
    var x: usize = 0
    var y: usize = 5
    while x < y {
      val s = a[x]
      a[x] = a[y]
      a[y] = s
      x += 1
      y -= 1
    }
    return false
  }
  var j: usize = 5
  while a[j] <= a[p] {
    j -= 1
  }
  val t = a[p]
  a[p] = a[j]
  a[j] = t
  var lo: usize = p + 1
  var hi: usize = 5
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  return true
}
```
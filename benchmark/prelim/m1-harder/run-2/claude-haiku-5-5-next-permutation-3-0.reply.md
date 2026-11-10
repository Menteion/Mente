```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 0
  var has = false
  var k: usize = 0
  while k < 5 {
    if a[k] < a[k + 1] {
      p = k
      has = true
    }
    k += 1
  }
  if not has {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t1 = a[lo]
      a[lo] = a[hi]
      a[hi] = t1
      lo += 1
      hi -= 1
    }
    return false
  }
  var j: usize = 5
  while a[j] <= a[p] {
    j -= 1
  }
  val t2 = a[p]
  a[p] = a[j]
  a[j] = t2
  var lo2: usize = p + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val t3 = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = t3
    lo2 += 1
    hi2 -= 1
  }
  true
}
```
```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 5
  while p > 0 and a[p - 1] >= a[p] {
    p -= 1
  }
  if p == 0 {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val swapped = a[lo]
      a[lo] = a[hi]
      a[hi] = swapped
      lo += 1
      hi -= 1
    }
    return false
  }
  val i = p - 1
  var j: usize = 5
  while a[j] <= a[i] {
    j -= 1
  }
  val pivot = a[i]
  a[i] = a[j]
  a[j] = pivot
  var lo2: usize = p
  var hi2: usize = 5
  while lo2 < hi2 {
    val s = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = s
    lo2 += 1
    hi2 -= 1
  }
  true
}
```
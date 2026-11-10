```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var pivot: usize = 6
  var i: usize = 0
  while i < 5 {
    if a[i] < a[i + 1] {
      pivot = i
    }
    i += 1
  }
  if pivot == 6 {
    var p: usize = 0
    var q: usize = 5
    while p < q {
      val w = a[p]
      a[p] = a[q]
      a[q] = w
      p += 1
      q -= 1
    }
    return false
  }
  var j: usize = 5
  while a[j] <= a[pivot] {
    j -= 1
  }
  val t = a[pivot]
  a[pivot] = a[j]
  a[j] = t
  var lo: usize = pivot + 1
  var hi: usize = 5
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  true
}
```
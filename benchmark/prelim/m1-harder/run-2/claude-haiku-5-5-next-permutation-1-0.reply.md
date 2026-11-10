```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var pivot: usize = 0
  var found = false
  var i: usize = 0
  while i < 5 {
    if a[i] < a[i + 1] {
      pivot = i
      found = true
    }
    i += 1
  }
  if not found {
    var lo0: usize = 0
    var hi0: usize = 5
    while lo0 < hi0 {
      val t1 = a[lo0]
      a[lo0] = a[hi0]
      a[hi0] = t1
      lo0 += 1
      hi0 -= 1
    }
    return false
  }
  var j: usize = 5
  while a[j] <= a[pivot] {
    j -= 1
  }
  val s = a[pivot]
  a[pivot] = a[j]
  a[j] = s
  var lo1: usize = pivot + 1
  var hi1: usize = 5
  while lo1 < hi1 {
    val t2 = a[lo1]
    a[lo1] = a[hi1]
    a[hi1] = t2
    lo1 += 1
    hi1 -= 1
  }
  true
}
```
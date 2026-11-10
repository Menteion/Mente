```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found = false
  var pivot: usize = 0
  var i: usize = 0
  while i < 5 {
    if a[i] < a[i + 1] {
      pivot = i
      found = true
    }
    i += 1
  }
  if not found {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val tmp = a[lo]
      a[lo] = a[hi]
      a[hi] = tmp
      lo += 1
      hi -= 1
    }
    return false
  }
  var j: usize = pivot + 1
  var m: usize = pivot + 2
  while m < 6 {
    if a[m] > a[pivot] {
      j = m
    }
    m += 1
  }
  val tmp3 = a[pivot]
  a[pivot] = a[j]
  a[j] = tmp3
  var lo2: usize = pivot + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val tmp2 = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = tmp2
    lo2 += 1
    hi2 -= 1
  }
  true
}
```
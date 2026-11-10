```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var pivot: usize = 0
  var has = false
  var k: usize = 0
  while k < 5 {
    if a[k] < a[k + 1] {
      pivot = k
      has = true
    }
    k += 1
  }
  if not has {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t = a[lo]
      a[lo] = a[hi]
      a[hi] = t
      lo += 1
      hi -= 1
    }
    return false
  }
  var j: usize = 5
  while a[j] <= a[pivot] {
    j -= 1
  }
  val tmp = a[pivot]
  a[pivot] = a[j]
  a[j] = tmp
  var l: usize = pivot + 1
  var r: usize = 5
  while l < r {
    val s = a[l]
    a[l] = a[r]
    a[r] = s
    l += 1
    r -= 1
  }
  true
}
```
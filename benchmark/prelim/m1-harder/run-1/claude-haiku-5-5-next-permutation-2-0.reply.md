```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var k: usize = 5
  var found = false
  var pivot: usize = 0
  while k > 0 {
    k -= 1
    if not found and a[k] < a[k + 1] {
      found = true
      pivot = k
    }
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
  var j: usize = 6
  var chosen = false
  var pick: usize = 0
  while not chosen {
    j -= 1
    if a[j] > a[pivot] {
      chosen = true
      pick = j
    }
  }
  val keep = a[pivot]
  a[pivot] = a[pick]
  a[pick] = keep
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
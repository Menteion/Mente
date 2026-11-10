```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var pivot: usize[0..5] = 0
  var has = false
  var k: usize[0..6] = 0
  while k < 5 {
    if a[k] < a[k + 1] {
      pivot = k
      has = true
    }
    k += 1
  }
  var lo: usize[0..6] = 0
  var hi: usize[0..6] = 5
  if has {
    var j: usize[0..6] = pivot + 1
    var n: usize[0..=6] = pivot + 1
    while n < 6 {
      if a[n] > a[pivot] {
        j = n
      }
      n += 1
    }
    val t = a[pivot]
    a[pivot] = a[j]
    a[j] = t
    lo = pivot + 1
  }
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  has
}
```
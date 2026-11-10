```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found = false
  var p: usize = 0
  var k: usize = 0
  while k < 5 {
    if a[k] < a[k + 1] {
      p = k
      found = true
    }
    k += 1
  }
  var start: usize = 0
  if found {
    var j: usize = 5
    while a[j] <= a[p] {
      j -= 1
    }
    val s = a[p]
    a[p] = a[j]
    a[j] = s
    start = p + 1
  }
  var lo = start
  var hi: usize = 5
  while lo < hi {
    val tmp = a[lo]
    a[lo] = a[hi]
    a[hi] = tmp
    lo += 1
    hi -= 1
  }
  found
}
```
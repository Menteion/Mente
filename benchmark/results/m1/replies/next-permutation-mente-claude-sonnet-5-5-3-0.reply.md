```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: i64 = 4
  var found = false
  while i >= 0 {
    if a[i] < a[i + 1] {
      found = true
      break
    }
    i -= 1
  }
  var start: i64 = 0
  if found {
    var j: i64 = 5
    while a[j] <= a[i] {
      j -= 1
    }
    val t = a[i]
    a[i] = a[j]
    a[j] = t
    start = i + 1
  }
  var lo = start
  var hi: i64 = 5
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  found
}
```
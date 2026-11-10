```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 {
    if a[i - 1] < a[i] {
      break
    }
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while a[j] <= a[i - 1] {
      j -= 1
    }
    val t = a[i - 1]
    a[i - 1] = a[j]
    a[j] = t
  }
  var l: usize = i
  var r: usize = 5
  while l < r {
    val u = a[l]
    a[l] = a[r]
    a[r] = u
    l += 1
    r -= 1
  }
  i > 0
}
```
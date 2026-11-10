```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 0
  var pivot: usize = 0
  var found = false
  while p < 5 {
    if a[p] < a[p + 1] {
      pivot = p
      found = true
    }
    p += 1
  }
  if found {
    var q: usize = 5
    while a[q] <= a[pivot] {
      q -= 1
    }
    val s = a[pivot]
    a[pivot] = a[q]
    a[q] = s
  }
  var l: usize = if found { pivot + 1 } else { 0 }
  var r: usize = 5
  while l < r {
    val tmp = a[l]
    a[l] = a[r]
    a[r] = tmp
    l += 1
    r -= 1
  }
  found
}
```
```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var pivot: i32 = -1
  var i: i32 = 4
  while i >= 0 {
    if a[i] < a[i + 1] {
      pivot = i
      break
    }
    i -= 1
  }
  if pivot >= 0 {
    var j: i32 = 5
    while j > pivot {
      if a[j] > a[pivot] {
        break
      }
      j -= 1
    }
    val t = a[pivot]
    a[pivot] = a[j]
    a[j] = t
  }
  var lo: i32 = pivot + 1
  var hi: i32 = 5
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  pivot >= 0
}
```
The same three examples fail, and the return values are still right. Every write goes straight into the `mut` parameter `a`, and that is the one thing I haven't varied. I'll copy `a` into a local `var b`, do all the work on `b`, then assign the result back to `a` in one whole-array assignment.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found = false
  var pivot: usize = 0
  var i: usize = 0
  while i < 5 {
    if b[i] < b[i + 1] {
      pivot = i
      found = true
    }
    i += 1
  }
  var result = true
  if not found {
    var p: usize = 0
    var q: usize = 5
    while p < q {
      val t0 = b[p]
      b[p] = b[q]
      b[q] = t0
      p += 1
      q -= 1
    }
    result = false
  }
  if found {
    var j: usize = 5
    while b[j] <= b[pivot] {
      j -= 1
    }
    val t1 = b[pivot]
    b[pivot] = b[j]
    b[j] = t1
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val t2 = b[lo]
      b[lo] = b[hi]
      b[hi] = t2
      lo += 1
      hi -= 1
    }
  }
  a = b
  result
}
```
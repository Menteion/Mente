The three examples produce the right return values, but the `leaves` check fails for all of them. Since every example fails, the likely cause is how the `mut` array is written back, not the permutation logic. I rewrote the function to work on a local copy, `b`, and assign the result to `a` once at each exit. I can't confirm the cause from the output alone, so this is a best-effort fix.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found: bool = false
  var pivot: usize = 0
  var j: usize = 5
  while j > 0 and not found {
    if b[j - 1] < b[j] {
      found = true
      pivot = j - 1
    }
    j -= 1
  }
  if not found {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t1 = b[lo]
      b[lo] = b[hi]
      b[hi] = t1
      lo += 1
      hi -= 1
    }
    a = b
    return false
  }
  var s: usize = 5
  while s > pivot and b[s] <= b[pivot] {
    s -= 1
  }
  val t2 = b[pivot]
  b[pivot] = b[s]
  b[s] = t2
  var lo2: usize = pivot + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val t3 = b[lo2]
    b[lo2] = b[hi2]
    b[hi2] = t3
    lo2 += 1
    hi2 -= 1
  }
  a = b
  true
}
```
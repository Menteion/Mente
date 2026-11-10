The return values were already correct. The failures are in the rearranged array, so I now do all the work on a local copy `b` and assign it back to `a` once at the end.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var p: usize = 5
  var found: bool = false
  while p > 0 and not found {
    p -= 1
    if b[p] < b[p + 1] {
      found = true
    }
  }
  var start: usize = 0
  if found {
    var q: usize = 5
    while b[q] <= b[p] {
      q -= 1
    }
    val t1 = b[p]
    b[p] = b[q]
    b[q] = t1
    start = p + 1
  }
  var lo: usize = start
  var hi: usize = 5
  while lo < hi {
    val t2 = b[lo]
    b[lo] = b[hi]
    b[hi] = t2
    lo += 1
    hi -= 1
  }
  a = b
  found
}
```
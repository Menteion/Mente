The first two attempts gave the right return values but not the expected final array. This version does all the work on a local copy `b` and assigns it back to `a` as a whole before returning.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize[0..6] = 5
  while i > 0 and b[i - 1] >= b[i] {
    i -= 1
  }
  if i > 0 {
    val p: usize[0..6] = i - 1
    var j: usize[0..6] = 5
    while j > i and b[j] <= b[p] {
      j -= 1
    }
    val tmp = b[p]
    b[p] = b[j]
    b[j] = tmp
  }
  var lo: usize[0..6] = i
  var hi: usize[0..6] = 5
  while lo < hi {
    val t = b[lo]
    b[lo] = b[hi]
    b[hi] = t
    lo += 1
    hi -= 1
  }
  a = b
  i > 0
}
```
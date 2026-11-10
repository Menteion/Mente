I still can't see a defect, and the harness output has not changed. This version is a variation I hadn't tried: it works on a local copy with plain `usize` indices and writes the whole array back to `a` just before returning. By hand it gives the expected array and return value on all three examples, as before.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var i: usize = 5
  while i > 0 and b[i - 1] >= b[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while j > i and b[j] <= b[p] invariant j <= 5 {
      j -= 1
    }
    val tmp = b[p]
    b[p] = b[j]
    b[j] = tmp
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
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
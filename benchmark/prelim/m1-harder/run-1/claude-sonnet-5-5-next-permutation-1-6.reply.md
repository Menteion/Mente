I could not find what the interpreter objects to. The function returns the right bool in all three cases, and I traced each example by hand. They give `[1,2,3,4,6,5]`, `[1,2,3,4,5,6]` and `[1,4,0,2,3,5]`, which are the expected arrays. I have not found a change to the code that alters the failures, so this is a clean version of the same algorithm, not a fix.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize[0..6] = 5
  while i > 0 and a[i - 1] >= a[i] {
    i -= 1
  }
  var lo: usize[0..6] = 0
  var hi: usize[0..6] = 5
  if i > 0 {
    val p: usize[0..6] = i - 1
    var j: usize[0..6] = 5
    while j > i and a[j] <= a[p] {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
    a[j] = t
    lo = i
  }
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  i > 0
}
```
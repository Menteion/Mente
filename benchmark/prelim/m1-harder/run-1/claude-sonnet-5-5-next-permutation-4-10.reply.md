I have no new fix. I traced all three examples by hand again, and the function gives the expected array and return value each time. Nine versions with different structures have failed with identical output. I won't present another rewrite as if it were a fix, because I can't tell what the interpreter's final array looks like. This is the same algorithm, unchanged.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while j > i and a[j] <= a[p] invariant j <= 5 {
      j -= 1
    }
    val tmp = a[p]
    a[p] = a[j]
    a[j] = tmp
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val t = a[lo]
    a[lo] = a[hi]
    a[hi] = t
    lo += 1
    hi -= 1
  }
  i > 0
}
```
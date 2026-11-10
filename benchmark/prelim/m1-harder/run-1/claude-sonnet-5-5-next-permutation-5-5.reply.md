I traced all three examples by hand and each gives the expected return value and array. The reported "got" shows only the return value, so I can't see what the interpreter holds for `a` afterwards. I'm giving the plainest version that does the swap and the suffix reversal directly on `a`.

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
    while j > i and a[j] <= a[i - 1] {
      j -= 1
    }
    val t = a[i - 1]
    a[i - 1] = a[j]
    a[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
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
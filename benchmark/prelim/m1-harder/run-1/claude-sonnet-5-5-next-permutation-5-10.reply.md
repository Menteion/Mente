I can't fix this from the report. The function returns the right value on all three examples, but the report shows only that value. It never shows the array the interpreter holds after the call, so I can't tell what is wrong with `leaves`. A rewrite would be a guess, and I've already tried several structurally different versions with identical results. If the report can include the actual array after the call, I can correct it directly. I am leaving the function unchanged.

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
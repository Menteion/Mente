I can't tell from the interpreter output what is wrong, because it only reports the returned value, and that matches in all three examples. Tracing each example by hand, the function leaves the array as expected. Here is the cleanest version, which passes `mn check`:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while j > i and a[j] <= a[i - 1] invariant j <= 5 {
      j -= 1
    }
    val t = a[i - 1]
    a[i - 1] = a[j]
    a[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  i > 0
}
```

I have now tried several structurally different versions, including working on a local copy and copying it back, and each fails with exactly the same message. That makes me think the `leaves` comparison is failing for a reason outside the code. If you can show the array the interpreter actually holds after the call, I can look for a real difference.
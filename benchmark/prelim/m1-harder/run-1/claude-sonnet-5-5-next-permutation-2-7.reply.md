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
    val p = i - 1
    var j: usize = 5
    while a[j] <= a[p] {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
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

I'm submitting the same program again. Nothing in the report has changed, and it still gives me nothing to fix. The returned values match, and by hand the final arrays match too. I would rather not change working code on a guess. If you can tell me what array the interpreter holds after the call, I can use that.
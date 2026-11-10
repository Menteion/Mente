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

I'm resubmitting the same program, because I have no evidence that any change would fix it. The report is identical each time. I have traced all three examples by hand, and the code returns the expected value and leaves the expected array in each.

I won't guess at a rewrite just to look responsive. The report doesn't show the array the interpreter ended up with, so I can't say what differs from the expected `leaves` value. If you can show me that array, or tell me how the interpreter reads a `mut` argument after a call that returns a value, I can work from that.
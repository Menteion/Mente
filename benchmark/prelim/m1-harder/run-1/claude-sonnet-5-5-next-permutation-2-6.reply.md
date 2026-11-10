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

This is the same program again, and I still think it is correct. Hand traces of all three examples give the expected return value and the expected final array. Four structurally different versions have produced a byte-identical report. Each time the returned value matches and the report says nothing about the final array, so I have nothing to act on. A fifth rewrite would be a guess.

What would help:
- the array the interpreter actually holds after each call, or
- the rule it uses to read back a `mut` argument when the function also returns a value.
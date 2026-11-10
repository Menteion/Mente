I can't find the cause of the `leaves` mismatch from the output alone. The return values match, and my trace of the three examples gives the expected arrays. The harness reports only the return value, so I can't tell whether the interpreter copies the `mut` array back differently from what I assumed. This version keeps the same algorithm but avoids compound loop conditions and reuses no names, which narrows the possible causes.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 4
  var pivot_found = false
  var scanning = true
  while scanning {
    if a[p] < a[p + 1] {
      pivot_found = true
      scanning = false
    } else if p == 0 {
      scanning = false
    } else {
      p -= 1
    }
  }
  var start: usize = 0
  if pivot_found {
    var j: usize = 5
    while a[j] <= a[p] {
      j -= 1
    }
    val swap_tmp = a[p]
    a[p] = a[j]
    a[j] = swap_tmp
    start = p + 1
  }
  var lo: usize = start
  var hi: usize = 5
  while lo < hi {
    val rev_tmp = a[lo]
    a[lo] = a[hi]
    a[hi] = rev_tmp
    lo += 1
    hi -= 1
  }
  pivot_found
}
```
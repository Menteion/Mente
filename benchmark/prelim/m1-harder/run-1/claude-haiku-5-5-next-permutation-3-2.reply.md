I've now submitted three versions, and the harness reports the same failure each time: the return values match, and the `leaves` check fails. The function body is not the cause. Tracing the three examples by hand gives the expected arrays for each. The likely cause is that the examples pass array literals as `mut` arguments, as in `mut [1, 2, 3, 4, 5, 6]`. A literal is not a place, so there is nothing for the copy-back to write into. If the harness is checking `leaves` on literal arguments, no function body can pass it. Checking the harness's handling of `mut` literal arguments would confirm this, and the examples may need to pass a `var` instead.

The function below is the correct algorithm, unchanged from the last version:

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
I can't fix this from the function body. This is the fifth submission, and every one has returned the same results: the return values are correct, and the `leaves` check fails. I hand-traced all three examples against the spec, and each gives the expected array. I'm not sure what the harness does with `mut` literals or how it reads `leaves`, so I'm not going to keep rewording the same algorithm.

What would settle it:
- Run the `probe` I gave earlier, with `v` declared as a `var`. If it returns `true`, the examples pass literals, and the harness or the examples need to change. If it returns `false`, the interpreter isn't copying `mut` arguments back, which is a bug to report there.
- Check how the harness reads `leaves`. Does it read the argument's place after the call, or the value the function returned?

Here is the function again, unchanged, so you can run it under whichever fix you choose:

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
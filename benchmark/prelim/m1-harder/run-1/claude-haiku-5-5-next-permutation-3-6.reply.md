I'm going to stop revising the body, because it cannot change this result. The return values match on all three examples, and I hand-checked the `leaves` arrays against the spec. The failure is in the `leaves` check, which the harness performs on arguments that are array literals.

I need one of these from you before I can make progress:
- The output of the `probe` with a `var` argument. This tells us whether the interpreter copies `mut` arguments back at all.
- Or the harness's rule for `leaves` on literal `mut` arguments. If the examples have to change, that is a change to the spec's examples, not to the function, and it's your call.

Here is the function, unchanged:

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
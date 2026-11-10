I can't find a change to the function that fixes this. Five submissions have now failed the same three `leaves` checks, and the return values are correct every time. The function body traces correctly on all three examples, so I'm not going to submit another rewrite that I expect to fail the same way.

What I can't tell from the feedback is what the interpreter leaves in the `mut` array after each call. That determines whether the problem is in the function or in the call form. A quick probe would settle it: a function that sets `a[0] = 9` on a `mut` parameter and returns `true`, called on a `var` bound to an array, not a literal. If the variable comes back with `9` in its first slot, write-back works and the examples' literal arguments are the problem. If it doesn't, the `mut` write-back itself is the problem, and no function body can meet these examples.

Please send me one of these:

1. The probe result, or the value the interpreter reports for the first example's array after the call.
2. Confirmation of whether `mut` arguments must be variables. If they must, the examples need to bind the array first, and I'll write the function to match.

Here is the function, unchanged from the last submission, since its logic is already correct:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found: bool = false
  var pivot: usize = 0
  var j: usize = 5
  while j > 0 and not found {
    if a[j - 1] < a[j] {
      found = true
      pivot = j - 1
    }
    j -= 1
  }
  if not found {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t1 = a[lo]
      a[lo] = a[hi]
      a[hi] = t1
      lo += 1
      hi -= 1
    }
    return false
  }
  var s: usize = 5
  while s > pivot and a[s] <= a[pivot] {
    s -= 1
  }
  val t2 = a[pivot]
  a[pivot] = a[s]
  a[s] = t2
  var lo2: usize = pivot + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val t3 = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = t3
    lo2 += 1
    hi2 -= 1
  }
  true
}
```
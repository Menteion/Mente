I can't fix this from the function body. The same three failures have now come back unchanged across three versions, so I'm not going to keep rewriting it.

The feedback shows only that `leaves` didn't match, not what value the interpreter got. Without that, I can't tell whether the problem is in the function or in how the harness passes and compares `mut` arguments.

The example calls pass array literals to `mut` parameters. The card says a `mut` parameter "receives a place", and a literal isn't one. That's the most likely suspect, but I can't confirm it from here.

What would unblock this:

1. The actual `leaves` value the interpreter reports for the first example. That would show whether the array is unchanged, partly changed, or changed differently from what I traced.
2. Confirmation of whether `mut` arguments must be variables. If they must, the examples need to bind the array to a `var` first, and I can write the function to match that.

The function's logic traces correctly on all three examples, so I'm not changing it. Here it is again, unchanged from the last version:

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
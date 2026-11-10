I traced all three examples through this function and each one produces the expected array and return value, so I don't think the function is the problem. The first version wrote directly to `a` and failed the same way, which points away from the copy-back.

I suspect the harness. The examples pass array literals as `mut` arguments, as in `next_permutation(mut [1, 2, 3, 4, 5, 6])`. The card says a `mut` argument must be a place, a variable that can be copied back into. If the reference interpreter runs a literal and then discards the copy-back, the leaves check would fail while the return value still matches, which is what the output shows. I can't see the harness's leaves value, so this is an inference.

The function below is the same as before. It is correct under the card's semantics:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var p: usize = 5
  var found: bool = false
  while p > 0 and not found {
    p -= 1
    if b[p] < b[p + 1] {
      found = true
    }
  }
  var start: usize = 0
  if found {
    var q: usize = 5
    while b[q] <= b[p] {
      q -= 1
    }
    val t1 = b[p]
    b[p] = b[q]
    b[q] = t1
    start = p + 1
  }
  var lo: usize = start
  var hi: usize = 5
  while lo < hi {
    val t2 = b[lo]
    b[lo] = b[hi]
    b[hi] = t2
    lo += 1
    hi -= 1
  }
  a = b
  found
}
```

To confirm the cause, run the examples with a variable as the argument:

```text
var v: [6]i32 = [1, 2, 3, 4, 5, 6]
next_permutation(mut v)   // then check v == [1, 2, 3, 4, 6, 5]
```

If `v` holds the expected array, the function is correct and the fault is in how the harness passes literals to `mut` parameters. If `v` doesn't, the copy-back isn't working for this call, and the reference interpreter's `mut` semantics need checking against the card's description.
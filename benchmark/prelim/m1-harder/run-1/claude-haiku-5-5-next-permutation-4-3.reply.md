I can't find a fault in the function, and I don't think another rewrite will change these results. The return values match on all three examples, and the array-leaves check failed the same way for three different implementations: writing to `a` directly, copying to `b` and assigning back, and the one above. A fix that only changes the function's internals should have moved at least one result.

The examples pass array literals to `mut` parameters. The card says a `mut` argument must be a place, and a literal isn't one. I can't see what the reference interpreter stores in `leaves` for a literal argument, so this is a guess, but it fits the evidence better than any problem in the algorithm.

Here is the function again. It's logically the same as my last two versions, and I haven't changed it:

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

To tell whether the function or the harness is at fault, call it with a variable instead of a literal:

```text
var v: [6]i32 = [1, 2, 3, 4, 5, 6]
next_permutation(mut v)
```

If `v` afterwards holds `[1, 2, 3, 4, 6, 5]`, the function is correct and the harness is discarding the copy-back for literal arguments. The examples should then be written with a variable, or the harness should be fixed. If `v` doesn't change, the `mut` copy-back itself is failing, and that's a bug in the interpreter, not in this function.
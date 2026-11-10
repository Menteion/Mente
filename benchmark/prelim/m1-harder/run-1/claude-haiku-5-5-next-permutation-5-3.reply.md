I can't find a fault in the function, and I don't think another rewrite will change the result. The three failures show the same pattern: the return value is right, and the array left in `a` is wrong, even for the simple first example, where the only change is one swap. Four different forms of the same algorithm produced the same output, so the cause is probably outside the function body.

The card says a `mut` argument "receives a place", and the test calls pass literals such as `mut [1, 2, 3, 4, 5, 6]`. A literal is not a place. If the harness writes the `mut` argument back to a place, a literal has nowhere to go, so `leaves` would never match. I can't confirm that without the interpreter's behaviour, so I haven't changed the logic.

I traced the function by hand on all three examples, and it produces the expected arrays:
- `[1,2,3,4,5,6]` gives pivot 4, swap to `[1,2,3,4,6,5]`, return `true`.
- `[6,5,4,3,2,1]` has no pivot, so it reverses to `[1,2,3,4,5,6]` and returns `false`.
- `[1,3,5,4,2,0]` gives pivot 1, swap to `[1,5,3,4,2,0]`... then the suffix reversal gives `[1,4,0,2,3,5]` and returns `true`.

The function below uses the copy-into-local form. It is correct by the card's rules and the trace above.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found = false
  var pivot: usize = 0
  var i: usize = 0
  while i < 5 {
    if b[i] < b[i + 1] {
      pivot = i
      found = true
    }
    i += 1
  }
  var result = true
  if not found {
    var p: usize = 0
    var q: usize = 5
    while p < q {
      val t0 = b[p]
      b[p] = b[q]
      b[q] = t0
      p += 1
      q -= 1
    }
    result = false
  }
  if found {
    var j: usize = 5
    while b[j] <= b[pivot] {
      j -= 1
    }
    val t1 = b[pivot]
    b[pivot] = b[j]
    b[j] = t1
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val t2 = b[lo]
      b[lo] = b[hi]
      b[hi] = t2
      lo += 1
      hi -= 1
    }
  }
  a = b
  result
}
```

To test whether the harness is at fault, run the examples with a named `var` as the `mut` argument, for example `var v: [6]i32 = [1, 2, 3, 4, 5, 6]` then `next_permutation(mut v)` and check `v`. If that passes, the fault is in how the harness treats literal `mut` arguments, and the function is fine as written.
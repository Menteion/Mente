I can't find a bug in the function, and I've been returning the same logic under different layouts. Four different implementations, including one that writes straight to `a` and one that unrolls every swap, all fail the same way. That points away from the body and toward how the `leaves` check reads back the `mut` argument.

Two things would help narrow it down:

1. **Does the harness support a literal as a `mut` argument?** The examples pass `mut [1, 2, 3, 4, 5, 6]`, a literal rather than a variable. The card describes `mut` as taking a place, as in `swap(mut v, 0, 3)` with `v` declared as a `var`. If literals have no place to write back to, `leaves` would never match even when the function is correct.
2. **What does `leaves` actually compare?** The output shows only `got true` and `got false`. If the harness can print the array it reads back after the call, I can see whether it is unchanged, partly changed, or mismatched.

If the harness only accepts variables, I can't fix the examples from here, since they're fixed. The function below is the correct version, unchanged from my previous reply, and it should pass once the `mut` write-back works.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found = false
  var pivot: usize = 0
  if b[0] < b[1] {
    found = true
    pivot = 0
  }
  if b[1] < b[2] {
    found = true
    pivot = 1
  }
  if b[2] < b[3] {
    found = true
    pivot = 2
  }
  if b[3] < b[4] {
    found = true
    pivot = 3
  }
  if b[4] < b[5] {
    found = true
    pivot = 4
  }
  var result = true
  if not found {
    val r0 = b[0]
    b[0] = b[5]
    b[5] = r0
    val r1 = b[1]
    b[1] = b[4]
    b[4] = r1
    val r2 = b[2]
    b[2] = b[3]
    b[3] = r2
    result = false
  }
  if found {
    var pick: usize = 0
    if pivot < 1 and b[1] > b[pivot] {
      pick = 1
    }
    if pivot < 2 and b[2] > b[pivot] {
      pick = 2
    }
    if pivot < 3 and b[3] > b[pivot] {
      pick = 3
    }
    if pivot < 4 and b[4] > b[pivot] {
      pick = 4
    }
    if pivot < 5 and b[5] > b[pivot] {
      pick = 5
    }
    val keep = b[pivot]
    b[pivot] = b[pick]
    b[pick] = keep
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val tmp = b[lo]
      b[lo] = b[hi]
      b[hi] = tmp
      lo += 1
      hi -= 1
    }
  }
  a = b
  result
}
```
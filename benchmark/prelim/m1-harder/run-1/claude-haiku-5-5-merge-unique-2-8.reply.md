I'm stuck, and submitting the same function again won't help. The counts pass and the leaves fail in every run, and I can't see the leaves the interpreter produced.

The likely cause is in the examples, not the merge logic. The card says a `mut` parameter "receives a place", and the examples pass a literal, `mut [0, 0, 0, ...]`. A literal isn't a place, so the interpreter may have no variable for the copy-back to write into. That would explain why the leaves never match whatever the function does. I can't change the examples, so this needs a decision from whoever owns them: either the harness should pass a `var` holding the array, or the card should say whether literals are allowed for `mut` arguments.

To confirm this, please run the probe from my earlier message with the `mut` argument as a `var`, for example `var v: [4]i32 = [0, 0, 0, 0]` followed by `probe(mut v)`. If the leaves come back as `[7, 0, 0, 9]`, the literal form is the problem, not the function. If they're still unchanged, send me the interpreter's full output.

Here is the function in its current form, since the harness needs a block to run:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
  while i < 8 or j < 8 {
    guard k < 16 else {
      unreachable("at most 16 distinct values are written")
    }
    val v: i32 = if i < 8 {
      if j < 8 {
        if a[i] <= b[j] { a[i] } else { b[j] }
      } else {
        a[i]
      }
    } else {
      b[j]
    }
    val fresh = if k == 0 { true } else { last != v }
    if fresh {
      out[k] = v
      last = v
      k += 1
    }
    if i < 8 {
      if a[i] == v {
        i += 1
      }
    }
    if j < 8 {
      if b[j] == v {
        j += 1
      }
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values are written")
  }
  k
}
```
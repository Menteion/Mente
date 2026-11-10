I've now submitted several versions with the same logic, and the leaves still fail while the counts pass. Each new variant has been a guess, so I'll stop changing the algorithm until I have better evidence.

What I know: the count is right in both examples, and tracing the code by hand gives the expected leaves. What I don't know is what the interpreter does with the `mut` array after the call. I can't see the leaves it produced, so I can't tell whether the writes are lost, applied to the wrong positions, or compared against something else.

Two things would settle it:

1. The actual leaves array the interpreter printed for each example, not only the expected value.
2. A probe run separately, not as a `mente` block in this reply, which would confuse the harness again. Write a function that sets `out[0] = 7` and `out[3] = 9` on a `mut [4]i32` parameter and returns 2. Call it with `mut [0, 0, 0, 0]`. If the leaves are `[7, 0, 0, 9]`, element writes work, and the problem is specific to this function. If the leaves are unchanged, the interpreter isn't copying `mut` arrays back for literal arguments, and no change to the merge code will fix the examples.

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
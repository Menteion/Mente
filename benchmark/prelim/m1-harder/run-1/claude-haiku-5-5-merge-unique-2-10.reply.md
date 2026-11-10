I've now traced this function by hand through both examples several times, and it produces the expected leaves each time. I also can't see anything in the feedback that points to a bug in the Mente code, so I'm not going to keep rewording it.

The likely causes are outside the function:

- **The `mut` argument is a literal.** The card says a `mut` parameter receives a place. The examples pass `mut [0, 0, ...]`, which isn't one. That may be why the interpreter never writes the result back for comparison.
- **The leaves comparison may be wrong in the harness.** The got-leaves value never appears in the feedback, so I can't check what the interpreter compared against.

Two things would settle it: run the probe with a `var` argument (`var v: [4]i32 = [0, 0, 0, 0]` then `probe(mut v)`), and check how the harness reads back `mut` arguments. If the probe works with a `var` and the examples still fail, the examples or the harness need fixing, and I'd want to see the interpreter's output before changing anything else.

Here is the function once more, since the harness needs a block to score:

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
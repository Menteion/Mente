I haven't changed the function, because the same failure has now come back across five different versions of the body. The count and the trace are correct, so more body edits won't help.

One possibility I haven't ruled out is the test call itself. The card says a `mut` parameter "receives a place, marked at the call", and in these examples the `mut` argument is an array literal, not a variable. If the interpreter only writes back to a place, the leaves check can't pass with any body. Before I change the function again, could you check whether literal `mut` arguments are supported, or re-run the examples with a `var` holding the array passed as `mut v`? If the leaves then match, the function is correct as it stands.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
  var seen = false
  while i + j < 16 {
    var v: i32 = 0
    if i < 8 {
      if j < 8 {
        if a[i] <= b[j] {
          v = a[i]
          i += 1
        } else {
          v = b[j]
          j += 1
        }
      } else {
        v = a[i]
        i += 1
      }
    } else {
      v = b[j]
      j += 1
    }
    if not seen or v != last {
      res[k] = v
      k += 1
      last = v
      seen = true
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  out = res
  return k
}
```
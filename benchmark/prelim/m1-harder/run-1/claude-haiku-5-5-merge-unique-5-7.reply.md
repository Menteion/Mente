I've now submitted the same function several times, and the leaves check keeps failing. Another rewrite won't change that, so I'm stopping here rather than trying a seventh variant.

What I know: the return values are correct, and my hand trace of both examples gives the expected leaves. The remaining explanation is the harness. The examples pass an array literal to `mut`, but the card says a `mut` argument is a place. I can't confirm that from the output I have, and I can't rewrite the examples myself.

Two things would settle it:
1. Run the first example with a variable: `var v: [16]i32 = [0, 0, ...]`, then `merge_unique(a, b, mut v)`, and show the leaves value.
2. If literal `mut` arguments are unsupported, change the examples to use a variable.

Here is the function as it stands:

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
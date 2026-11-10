I don't think another rewrite of the function will fix this. The return values are correct in both examples, and I've checked the leaves by hand against the expected values. Four different versions of the function produced the same failure, which points away from the algorithm. The likely cause is how the harness passes a literal as the `mut` argument. The card says a `mut` argument receives a place, and a literal may not be one.

Here is the function written the most direct way, with element assignment straight into `out`, as the card allows:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var v: i32 = 0
  var fresh = true
  while i < 8 or j < 8 {
    if i < 8 and j < 8 {
      if a[i] <= b[j] {
        v = a[i]
        i += 1
      } else {
        v = b[j]
        j += 1
      }
    } else if i < 8 {
      v = a[i]
      i += 1
    } else {
      v = b[j]
      j += 1
    }
    fresh = true
    if k > 0 {
      if out[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      out[k] = v
      k += 1
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  k
}
```

To tell whether the problem is the harness or the function, please run the first example with the array in a `var`, for example `var o: [16]i32 = [0, ...]` and then `merge_unique(a, b, mut o)`, and check `o` afterward. If `o` is correct, the function is fine and the harness is mishandling literal `mut` arguments. If `o` is wrong, send me the value the interpreter returns for `o` and I'll track down the difference.
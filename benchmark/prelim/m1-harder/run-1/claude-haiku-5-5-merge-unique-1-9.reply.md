I can't make progress by changing the function. Every version I've written returns the right count and, by hand-trace, writes the expected values. The report is identical each time and shows only the return value, so I can't see what the interpreter stored in `out`.

The function, unchanged:

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

The likely cause is outside the function: the examples pass literal arrays as `mut` arguments, and the card says a `mut` argument receives a place. If the interpreter doesn't copy the array back into a literal, these two examples can't pass with any implementation. Two checks would confirm this:

1. Run the first example with `var o: [16]i32 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]` and then `merge_unique(a, b, mut o)`. Check whether `o` holds the expected leaves.
2. Paste the interpreter's full output for those examples, including the actual leaves value, not only the `got` line.

If the `var` test passes, the function is correct and the harness is at fault. If it fails, send me the array it produces and I'll fix the specific positions.
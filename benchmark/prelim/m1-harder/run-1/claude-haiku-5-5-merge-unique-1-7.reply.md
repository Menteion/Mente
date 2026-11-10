The return values are right in both failing examples, and the dedup logic must be right too, since a wrong dedup would change the count. The only thing left that differs is the contents of `out` after the call. A change to the function's logic can't fix that, so I'm not going to send another rewrite.

Here is the function as it stands:

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

To find the cause, I need two things from the interpreter:

1. The full output for the first example, including the `leaves` value it actually produced. The report prints `got 12` but not the array.
2. The result of the `var` test: `var o: [16]i32 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]`, then `merge_unique(a, b, mut o)`, then the contents of `o`.

If `o` is correct, the function works and the harness is not writing back literal `mut` arguments. If `o` is wrong, the array it contains will show which positions differ, and I can fix those directly.
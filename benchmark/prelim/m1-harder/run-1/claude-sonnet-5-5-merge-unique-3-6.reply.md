I still can't see a bug. By hand, this version returns 12 and 8 and leaves `out` as the examples expect, and the FAIL line again shows only the returned value, which matches. This time I'm submitting the single-pass variant, which differs structurally from the earlier ones: it takes the smaller head each round and skips a value equal to the last one written.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i >= 8 {
      guard j < 8 else {
        unreachable("loop condition needs j < 8 here")
      }
      v = b[j]
      j += 1
    } else if j >= 8 {
      v = a[i]
      i += 1
    } else if a[i] <= b[j] {
      v = a[i]
      i += 1
    } else {
      v = b[j]
      j += 1
    }
    var fresh: bool = true
    if k > 0 {
      if out[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = v
      k += 1
    }
  }
  k
}
```
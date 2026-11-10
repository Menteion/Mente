The return values are correct in both failing examples, and only the array contents differ. I'll stop writing element by element into the `mut` parameter during the loop. The function now builds the result in a local array and assigns it to `out` once, at the end.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
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
      if res[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      res[k] = v
      k += 1
    }
  }
  out = res
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  k
}
```
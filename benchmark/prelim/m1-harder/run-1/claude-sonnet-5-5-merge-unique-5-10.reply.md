I still can't find a fault: the count matches, and a hand-trace of both examples gives the expected contents of `out`. In this version every change to `out` happens through one whole-array assignment just before the return. The merge itself is built in a local array.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var merged: [16]i32 = out
  var ia: usize[0..=8] = 0
  var ib: usize[0..=8] = 0
  var count: usize[0..=16] = 0
  var started = false
  var prev: i32 = 0
  var cur: i32 = 0
  while ia < 8 or ib < 8 {
    if ia < 8 {
      if ib < 8 {
        if a[ia] <= b[ib] {
          cur = a[ia]
          ia += 1
        } else {
          cur = b[ib]
          ib += 1
        }
      } else {
        cur = a[ia]
        ia += 1
      }
    } else {
      if ib < 8 {
        cur = b[ib]
        ib += 1
      }
    }
    if not started or cur != prev {
      if count < 16 {
        merged[count] = cur
        count += 1
      }
      prev = cur
      started = true
    }
  }
  out = merged
  return count
}
```
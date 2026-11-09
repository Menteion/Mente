```mente
pub fn lower_bound(a: [16]i32, x: i32) -> usize[0..=16] {
  var lo: usize = 0
  var hi: usize = 16
  while lo < hi invariant hi <= 16 {
    val mid: usize = lo + (hi - lo) / 2
    if a[mid] < x {
      lo = mid + 1
    } else {
      hi = mid
    }
  }
  guard lo <= 16 else {
    unreachable("lo never passes 16")
  }
  lo
}
```
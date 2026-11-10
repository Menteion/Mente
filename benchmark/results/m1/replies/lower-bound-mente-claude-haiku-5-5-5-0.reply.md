```mente
pub fn lower_bound(a: [16]i32, x: i32) -> usize[0..=16] {
  var lo: usize = 0
  var hi: usize = 16
  while lo < hi {
    val mid = (lo + hi) / 2
    guard mid < 16 else {
      unreachable("mid is below hi, and hi is at most 16")
    }
    if a[mid] < x {
      lo = mid + 1
    } else {
      hi = mid
    }
  }
  guard lo <= 16 else {
    unreachable("lo never exceeds hi, and hi starts at 16")
  }
  lo
}
```
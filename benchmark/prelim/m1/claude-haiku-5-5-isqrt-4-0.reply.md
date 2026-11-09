```mente
pub fn isqrt(n: u64) -> u64 {
  var r: u64 = 0
  var p: u64 = 2147483648
  while p > 0 {
    val c: u64 = r + p
    if c * c <= n {
      r = c
    }
    p = p / 2
  }
  r
}
```
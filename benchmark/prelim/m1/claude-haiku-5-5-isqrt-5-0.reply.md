```mente
pub fn isqrt(n: u64) -> u64 {
  var r: u64 = 0
  var bit: u64 = 2147483648
  while bit > 0 {
    val c = r + bit
    if c * c <= n {
      r = c
    }
    bit >>= 1
  }
  r
}
```
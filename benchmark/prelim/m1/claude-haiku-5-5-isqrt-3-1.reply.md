The bug was the bit step: the algorithm walks powers of four, so `bit` must shift by 2, not 1. With the shift corrected, `isqrt(16)` gives 4 and `isqrt(15)` gives 3.

```mente
pub fn isqrt(n: u64) -> u64 {
  var x: u64 = n
  var bit: u64 = 1 << 62
  var r: u64 = 0
  while bit > x {
    bit >>= 2
  }
  while bit != 0 {
    if x >= r + bit {
      x -= r + bit
      r = (r >> 1) + bit
    } else {
      r >>= 1
    }
    bit >>= 2
  }
  r
}
```
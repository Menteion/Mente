```mente
pub fn isqrt(n: u64) -> u64 {
  val nn: u128 = n
  var lo: u64 = 0
  var hi: u64 = 4294967295
  while lo < hi {
    val mid: u64 = lo + (hi - lo + 1) / 2
    val sq: u128 = mid * mid
    if sq <= nn {
      lo = mid
    } else {
      hi = mid - 1
    }
  }
  lo
}
```
```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  if x < lo {
    return 0
  }
  if x > hi {
    return n - 1
  }
  val offset: u64 = x - lo
  val width: u64 = hi - lo + 1
  val buckets: u64 = n
  val scaled: u64 = offset * buckets
  val bkt: u64 = scaled / width
  guard bkt < buckets else {
    unreachable("offset is below width")
  }
  bkt
}
```
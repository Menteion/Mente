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
  val offset: i64 = x - lo
  val width: i64 = hi - lo + 1
  val wide: i64 = offset * n
  val idx: u32 = wide / width
  guard idx < n else {
    unreachable("offset is below width, so the bucket is below n")
  }
  idx
}
```
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
  val span: i64 = hi - lo + 1
  val scaled: i64 = offset * n
  val index: i64 = scaled / span
  guard index >= 0 and index < 64 and index < n else {
    unreachable("offset is below span, so index is below n")
  }
  val b: u32[0..64] = index
  b
}
```
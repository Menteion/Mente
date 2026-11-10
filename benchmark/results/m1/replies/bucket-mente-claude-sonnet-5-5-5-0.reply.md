```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  guard x >= lo else {
    return 0
  }
  guard x <= hi else {
    return n - 1
  }
  val offset: i64 = x - lo
  val width: i64 = hi - lo + 1
  val r: i64 = offset * n / width
  guard r >= 0 and r < n else {
    unreachable("offset is below width, so r is below n")
  }
  r
}
```
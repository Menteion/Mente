```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  guard x > lo else {
    return 0
  }
  guard x < hi else {
    return n - 1
  }
  val dx: i64 = x - lo
  val span: i64 = hi - lo + 1
  val count: i64[1..=64] = n
  val b: i64 = dx * count / span
  guard b < count else {
    unreachable("dx < span, so the bucket is below n")
  }
  val r: u32[0..64] = b
  r
}
```
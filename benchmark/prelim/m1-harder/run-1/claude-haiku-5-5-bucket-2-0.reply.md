```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  var v: i64 = x
  if v < lo {
    v = lo
  }
  if v > hi {
    v = hi
  }
  val off: i64 = v - lo
  val width: i64 = hi - lo + 1
  val scaled: i128 = off * n
  val q: i128 = scaled / width
  guard q < n else {
    unreachable("off is at most width - 1, so the bucket is below n")
  }
  guard q < 64 else {
    unreachable("n is at most 64, so the bucket is below 64")
  }
  q
}
```
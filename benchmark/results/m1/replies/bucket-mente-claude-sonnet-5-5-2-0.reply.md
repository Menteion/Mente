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
  val width: i64 = hi - lo + 1
  val offset: i64 = x - lo
  val q: i64 = offset * n / width
  guard q >= 0 else {
    unreachable("offset and width are positive")
  }
  guard q < n else {
    unreachable("offset is below width")
  }
  q
}
```
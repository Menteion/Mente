```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  val c: i32 = if x < lo { lo } else if x > hi { hi } else { x }
  val offset: i64 = c - lo
  val width: i64 = hi - lo + 1
  val count: i64 = n
  val b: i64 = offset * count / width
  guard b >= 0 and b < count else {
    unreachable("offset is within the range, so the bucket is below n")
  }
  guard b < 64 else {
    unreachable("n is at most 64, so the bucket is below 64")
  }
  b
}
```
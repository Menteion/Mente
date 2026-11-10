```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  val low: i64 = lo
  val high: i64 = hi
  val v: i64 = x
  val c: i64 = if v < low { low } else if v > high { high } else { v }
  val offset: i64 = c - low
  val width: i64 = high - low + 1
  val n64: i64 = n
  val b: i64 = offset * n64 / width
  val r: u32 = b
  guard r < n else {
    unreachable("offset times n over width is below n")
  }
  r
}
```
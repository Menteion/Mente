```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  val c: i32 = if x < lo { lo } else if x > hi { hi } else { x }
  val offset: i64 = c - lo
  val width: i64 = hi - lo + 1
  val nn: i64 = n
  val b: i64 = offset * nn / width
  guard b >= 0 and b < nn else {
    unreachable("offset is within the range, so the bucket is below n")
  }
  val bkt: u32[0..64] = b
  bkt
}
```
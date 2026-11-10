```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  val c: i64 = if x < lo { lo } else if x > hi { hi } else { x }
  val width: i64 = hi - lo + 1
  val b: i64 = (c - lo) * n / width
  guard b < n else {
    unreachable("bucket index is below n")
  }
  b
}
```
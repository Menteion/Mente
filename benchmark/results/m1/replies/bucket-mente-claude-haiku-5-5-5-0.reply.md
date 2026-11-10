```mente
pub fn bucket(x: i32, lo: i32, hi: i32, n: u32[1..=64]) -> u32[0..64]
  requires lo < hi
  ensures result < n
{
  val c: i64 = if x < lo { lo } else if x > hi { hi } else { x }
  val offset: i64 = c - lo
  val width: i64 = hi - lo + 1
  val scaled: i64 = offset * n
  val idx: i64 = scaled / width
  guard idx >= 0 and idx < n else {
    unreachable("idx lies in 0..n")
  }
  idx
}
```
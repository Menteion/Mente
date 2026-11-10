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
  val width = hi - lo + 1
  val slot: u32 = (x - lo) * n / width
  guard slot < n else {
    unreachable("x <= hi, so (x - lo) * n is below width * n")
  }
  slot
}
```
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
  val r: u32 = (x - lo) * n / (hi - lo + 1)
  guard r < n else {
    unreachable("x is at most hi, so the bucket is below n")
  }
  r
}
```
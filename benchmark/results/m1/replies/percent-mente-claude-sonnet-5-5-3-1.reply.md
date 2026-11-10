```mente
pub fn percent(part: u32, whole: u32) -> u32[0..=100]
  requires whole > 0 and part <= whole
{
  val p: u32 = part * 100 / whole
  guard p <= 100 else {
    unreachable("part <= whole, so part * 100 / whole <= 100")
  }
  p
}
```
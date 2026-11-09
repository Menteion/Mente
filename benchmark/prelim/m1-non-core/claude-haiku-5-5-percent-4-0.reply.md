```mente
pub fn percent(part: u32, whole: u32) -> u32[0..=100]
  requires whole > 0 and part <= whole
{
  val p = part * 100 / whole
  guard p <= 100 else {
    unreachable("part <= whole bounds the quotient by 100")
  }
  p
}
```
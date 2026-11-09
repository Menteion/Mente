```mente
pub fn percent(part: u32, whole: u32) -> u32[0..=100]
  requires whole > 0 and part <= whole
{
  val scaled: u64 = part * 100
  val q: u64 = scaled / whole
  q
}
```
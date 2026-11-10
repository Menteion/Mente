```mente
pub fn percent(part: u32, whole: u32) -> u32[0..=100]
  requires whole > 0 and part <= whole
{
  val scaled: u128 = part * 100
  val q: u128 = scaled / whole
  guard q <= 100 else {
    unreachable("part <= whole bounds the quotient by 100")
  }
  q
}
```
```mente
/// No u8 is greater than 255, so the contract cannot hold for x == 255.
/// That case is left as an explicit todo (it traps) instead of returning a wrong value.
pub fn step_up(x: u8) -> u8
  ensures result > x
{
  guard x < 255 else {
    todo("no u8 is greater than 255; the signature cannot be satisfied for this input")
  }
  x + 1
}
```
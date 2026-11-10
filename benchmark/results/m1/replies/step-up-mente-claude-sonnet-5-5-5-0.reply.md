```mente
/// No u8 is greater than 255, so this contract cannot be met for x == 255.
/// That case traps explicitly rather than returning a wrong value.
pub fn step_up(x: u8) -> u8
  ensures result > x
{
  guard x < 255 else {
    todo("impossible for x == 255: no u8 is greater than 255")
  }
  x + 1
}
```
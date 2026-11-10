```mente
pub fn step_up(x: u8) -> u8
  ensures result > x
{
  // Impossible for x == 255: no u8 is greater than 255.
  // That input traps via todo rather than returning a wrong value.
  guard x < 255 else {
    return todo("no u8 is greater than 255; the signature cannot be met for x == 255")
  }
  x + 1
}
```
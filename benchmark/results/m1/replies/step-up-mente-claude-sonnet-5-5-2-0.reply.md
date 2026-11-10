```mente
/// No u8 is greater than 255, so this contract cannot hold for every input.
/// For x == 255 the function traps via `todo` instead of returning a wrong value.
pub fn step_up(x: u8) -> u8
  ensures result > x
{
  guard x < 255 else {
    todo("no u8 is greater than 255; the contract is unsatisfiable for x == 255")
  }
  x + 1
}
```
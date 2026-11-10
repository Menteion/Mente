This cannot be done as specified: for `x = 255` no `u8` is greater than `x`. The version below handles that one input with an explicit `todo` that traps, rather than returning a value that breaks the `ensures`.

```mente
pub fn step_up(x: u8) -> u8
  ensures result > x
{
  guard x < 255 else {
    todo("no u8 is greater than 255")
  }
  x + 1
}
```
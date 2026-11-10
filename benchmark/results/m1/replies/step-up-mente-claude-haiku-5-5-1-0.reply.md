No function can meet this spec. For `x = 255`, no `u8` is greater than 255, so `ensures result > x` cannot hold. The compiler would reject any body that returns a `u8` there, and `todo` or `unreachable` would trap, which the spec forbids.

Two versions compile. The first keeps the `u8` return type but excludes 255 from the input:

```mente
pub fn step_up(x: u8) -> u8
  requires x < 255
  ensures result > x
{
  x + 1
}
```

The second accepts every `u8` input by widening the return type:

```mente
pub fn step_up(x: u8) -> u16
  ensures result > x
{
  x + 1
}
```

Tell me which constraint you want to keep, and I'll adjust the function.
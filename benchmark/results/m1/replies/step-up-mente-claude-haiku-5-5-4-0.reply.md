No function can meet this spec. For `x = 255` there is no `u8` greater than `x`, so `ensures result > x` cannot hold on every input. Mente also gives no way out: `todo` and `unreachable` trap, which the spec forbids, and any returned value would violate the `ensures`.

The closest valid version adds a precondition, which narrows the input domain and so no longer covers every `u8`:

```mente
pub fn step_up(x: u8) -> u8
  requires x < 255
  ensures result > x
{
  x + 1
}
```

If the spec is meant to cover all inputs, it needs a different result type, such as `u16`, or a different return convention, such as an `Option`-like value, which Mente M1 does not have yet.
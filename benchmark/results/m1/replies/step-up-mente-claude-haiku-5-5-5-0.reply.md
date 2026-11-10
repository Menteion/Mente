This can't be written. For `x = 255` no `u8` is greater than 255, so `ensures result > x` cannot hold for every input. Any body either traps at `x + 1` or returns a value that breaks the contract, and the compiler will reject the contract if it can't prove it. The signature needs to change.

Two ways to make it satisfiable:

**Option 1: require a smaller input.** This changes the signature by adding a `requires`:

```mente
pub fn step_up(x: u8) -> u8
  requires x < 255
  ensures result > x
{
  x + 1
}
```

**Option 2: weaken the guarantee to `>=` and saturate at the top.** This keeps every input valid:

```mente
pub fn step_up(x: u8) -> u8
  ensures result >= x
{
  if x == 255 { x } else { x + 1 }
}
```

I haven't checked Option 2 with `mn check`, so I can't confirm the compiler accepts it. Tell me which contract you want and I'll write that version.
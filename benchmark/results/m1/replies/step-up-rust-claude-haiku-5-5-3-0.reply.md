No `u8` is greater than 255, so the postcondition cannot hold for `x = 255`. The function below satisfies `result > x` for every other input and does not panic. For `255` it returns `255`, which violates the stated guarantee and is the best that can be done under the no-trap requirement.

```rust
/// Ensures: `result > x`, where `result` is the value returned.
/// Impossible for x == 255: no u8 is greater than 255. In that case this
/// returns 255 (no trap), so the postcondition is violated only there.
pub fn step_up(x: u8) -> u8 {
    if x < u8::MAX { x + 1 } else { x }
}
```
```rust
/// Ensures: `result > x`, where `result` is the value returned.
///
/// NOTE: this contract cannot be met for `x == 255`, because no `u8` is greater
/// than `u8::MAX`. For every `x < 255` this returns `x + 1`, which is greater
/// than `x`. For `x == 255` it saturates and returns 255, so it does not panic,
/// but the postcondition `result > x` does not hold for that one input.
pub fn step_up(x: u8) -> u8 {
    x.saturating_add(1)
}
```
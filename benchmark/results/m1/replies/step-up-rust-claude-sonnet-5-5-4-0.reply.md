```rust
/// Ensures: `result > x`, where `result` is the value returned.
///
/// Note: this postcondition is impossible for `x == 255`, because no `u8`
/// is greater than `u8::MAX`. This implementation never traps and returns
/// `x + 1` for every `x < 255`. For `x == 255` it saturates to 255, so the
/// postcondition fails for that one input.
pub fn step_up(x: u8) -> u8 {
    x.saturating_add(1)
}
```
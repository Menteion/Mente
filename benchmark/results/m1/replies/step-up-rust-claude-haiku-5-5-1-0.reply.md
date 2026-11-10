```rust
/// Ensures: `result > x`, where `result` is the value returned.
///
/// This cannot hold for every `u8`. No `u8` is greater than 255, so for
/// `x == 255` the postcondition is unsatisfiable. This function returns 255
/// there instead of trapping, so the postcondition is violated for that one input.
/// For all `x < 255` it returns `x + 1`, which satisfies the postcondition.
pub fn step_up(x: u8) -> u8 {
    x.saturating_add(1)
}
```
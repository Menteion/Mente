```rust
/// Ensures: `result > x`, where `result` is the value returned.
///
/// NOTE: this contract cannot be met for x == 255, because no `u8` is greater
/// than `u8::MAX`. For every x < 255 this returns x + 1, which is greater than x.
/// For x == 255 it saturates and returns 255 rather than panicking, so the
/// postcondition `result > x` is violated for that single input.
pub fn step_up(x: u8) -> u8 {
    x.saturating_add(1)
}
```
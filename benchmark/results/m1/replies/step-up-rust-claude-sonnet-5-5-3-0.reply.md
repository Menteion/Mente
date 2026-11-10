This can't be done as specified. For `x = 255` no `u8` is greater than `x`, so no function with this signature can satisfy `result > x` for every input without trapping. The closest option is to saturate, which never panics and is correct for `0..=254`, but returns `255` for `255`, violating the postcondition there.

```rust
/// Ensures: `result > x` for all `x < 255`. For `x == 255` this is impossible
/// for a `u8`, so it saturates and returns 255.
pub fn step_up(x: u8) -> u8 {
    x.saturating_add(1)
}
```
This can't be done for every input. When `x == 255`, no `u8` is greater than `x`, so no function with this signature can meet the postcondition. The closest option that never panics is to saturate, which is correct for all inputs except 255:

```rust
pub fn step_up(x: u8) -> u8 {
    // For x == 255 no u8 greater than x exists; this returns 255, which violates `result > x`.
    x.saturating_add(1)
}
```

The alternatives are worse. `x + 1` panics on 255 with overflow checks on, and wrapping returns 0, which is less than `x`. Looping forever on 255 never returns a wrong value, but it is a hang rather than a real solution. To make the contract satisfiable, change the return type (for example to `u16` or `Option<u8>`) or add the precondition `x < 255`.
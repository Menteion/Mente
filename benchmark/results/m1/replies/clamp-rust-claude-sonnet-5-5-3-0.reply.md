```rust
/// Requires: `lo <= hi`.
/// Ensures: `lo <= result && result <= hi`, where `result` is the value returned.
pub fn clamp(x: i32, lo: i32, hi: i32) -> i32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
```
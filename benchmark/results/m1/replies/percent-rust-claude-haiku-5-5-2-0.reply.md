```rust
/// Requires: `whole > 0 && part <= whole`.
/// The result is in `0..=100`.
pub fn percent(part: u32, whole: u32) -> u32 {
    ((u64::from(part) * 100) / u64::from(whole)) as u32
}
```
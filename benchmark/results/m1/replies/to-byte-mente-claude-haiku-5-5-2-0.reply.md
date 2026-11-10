No `u8` can hold a value above 255, so no function with this signature can return `x` for every `u32` without trapping. The body below is the direct conversion. Mente checks the returned value against `u8`, so it traps when `x > 255`.

```mente
pub fn to_byte(x: u32) -> u8 = x
```
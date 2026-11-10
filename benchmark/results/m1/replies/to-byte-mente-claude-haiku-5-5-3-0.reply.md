No version of this function can meet all of the requirements. A `u8` holds 0 to 255, but a `u32` can hold up to 4294967295, so no `u8` can return `x` unchanged for every `u32` input. Returning `x` directly traps for any `x` above 255, which breaks the "never trap" rule. Any version that never traps has to change the value for those inputs, which breaks "loses nothing." The closest version that never traps saturates at 255:

```mente
pub fn to_byte(x: u32) -> u8 {
  guard x <= 255 else {
    return 255
  }
  x
}
```

If the caller must never pass a value above 255, a `requires x <= 255` contract would state that, but it changes the signature you specified.
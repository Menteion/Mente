No `u8` can hold the values 256 through 4294967295, so no function with this signature can return `x` unchanged for every `u32` input without trapping or losing information. The closest version below returns `x` for 0 to 255 and saturates to 255 above that. It does not preserve the value for larger inputs, so use it only if saturation is what you want. If you need a lossless conversion, the input or the return type has to change.

```mente
pub fn to_byte(x: u32) -> u8 {
  guard x <= 255 else {
    return 255
  }
  x
}
```
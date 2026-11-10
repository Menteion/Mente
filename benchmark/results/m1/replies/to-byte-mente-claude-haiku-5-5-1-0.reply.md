No function with this signature can return `x` unchanged for every `u32` input. A `u8` holds 0 to 255, and a value outside that range traps at run time, which also breaks "never trap." The request asks for two things that cannot both hold.

The version below never traps, but it saturates: inputs above 255 return 255, so information is lost for those inputs.

```mente
pub fn to_byte(x: u32) -> u8 {
  guard x <= 255 else {
    return 255
  }
  x
}
```

If you need a lossless conversion, the input must be limited to 0 to 255. That means changing the signature, for example to `x: u32[0..256]`, or making the function return a type that can hold every `u32`.
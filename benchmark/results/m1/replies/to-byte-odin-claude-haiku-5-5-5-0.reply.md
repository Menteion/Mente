```odin
// No u8 can hold every u32, so this keeps the low eight bits of x (x mod 256).
// The conversion never traps; values above 255 are truncated, not reported.
to_byte :: proc(x: u32) -> u8 {
	return u8(x)
}
```
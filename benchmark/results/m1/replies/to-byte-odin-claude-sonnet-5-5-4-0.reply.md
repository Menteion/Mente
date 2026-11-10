```odin
// Not lossless for every input: a u8 holds only 0..255, so for x > 255 the
// result keeps just the low 8 bits (x & 0xFF). The result equals x only when x <= 255.
// This never traps.
to_byte :: proc(x: u32) -> u8 {
	return u8(x)
}
```
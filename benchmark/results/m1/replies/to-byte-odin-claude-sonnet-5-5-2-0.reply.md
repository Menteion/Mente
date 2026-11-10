```odin
// Note: a u8 holds only 0..255, so for x > 255 this keeps the low 8 bits
// (x & 0xFF) and the result is not x itself. No u8 can represent those inputs.
to_byte :: proc(x: u32) -> u8 {
	return u8(x)
}
```
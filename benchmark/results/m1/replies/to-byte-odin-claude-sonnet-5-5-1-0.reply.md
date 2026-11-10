```odin
// Not possible for every input: a u8 holds only 0..255, so for x > 255 no
// u8 equals x. This returns x for x <= 255 and the low 8 bits (x mod 256)
// otherwise. It never traps. If the task guarantees x <= 255, the result is exactly x.
to_byte :: proc(x: u32) -> u8 {
	return u8(x)
}
```
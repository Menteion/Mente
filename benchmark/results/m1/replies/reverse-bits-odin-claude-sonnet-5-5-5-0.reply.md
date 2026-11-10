```odin
reverse_bits :: proc(x: u8) -> u8 {
	result: u8 = 0
	v := x
	for i := 0; i < 8; i += 1 {
		result = (result << 1) | (v & 1)
		v = v >> 1
	}
	return result
}
```
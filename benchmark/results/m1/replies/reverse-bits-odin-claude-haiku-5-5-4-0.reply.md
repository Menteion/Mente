```odin
reverse_bits :: proc(x: u8) -> u8 {
	v := x
	result: u8 = 0
	for i in 0..<8 {
		result = (result << 1) | (v & 1)
		v >>= 1
	}
	return result
}
```
```odin
reverse_bits :: proc(x: u8) -> u8 {
	result: u8 = 0
	for i in 0..<8 {
		if (x >> uint(i)) & 1 == 1 {
			result |= u8(1) << uint(7 - i)
		}
	}
	return result
}
```
reverse_bits :: proc(x: u8) -> u8 {
	result: u8 = 0
	v := x
	for _ in 0 ..< 8 {
		result = (result << 1) | (v & 1)
		v >>= 1
	}
	return result
}

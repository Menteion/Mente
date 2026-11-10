```odin
reverse_bits :: proc(x: u8) -> u8 {
	v := x
	r: u8 = 0
	for i in 0..<8 {
		r = (r << 1) | (v & 1)
		v >>= 1
	}
	return r
}
```
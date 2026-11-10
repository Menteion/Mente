```odin
isqrt :: proc(n: u64) -> u64 {
	r: u64 = 0
	for bit := 31; bit >= 0; bit -= 1 {
		c := r | (u64(1) << uint(bit))
		if c * c <= n {
			r = c
		}
	}
	return r
}
```
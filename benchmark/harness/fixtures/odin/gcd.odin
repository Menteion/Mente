gcd :: proc(a: u64, b: u64) -> u64 {
	x, y := a, b
	for y != 0 {
		x, y = y, x % y
	}
	return x
}

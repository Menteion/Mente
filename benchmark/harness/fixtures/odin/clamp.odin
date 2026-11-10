clamp :: proc(x: i32, lo: i32, hi: i32) -> i32 {
	if x < lo {
		return lo
	}
	if x > hi {
		return hi
	}
	return x
}

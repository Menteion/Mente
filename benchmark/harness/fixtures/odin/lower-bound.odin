lower_bound :: proc(a: [16]i32, x: i32) -> uint {
	for v, i in a {
		if v >= x {
			return uint(i)
		}
	}
	return 16
}

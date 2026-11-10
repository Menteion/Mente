index_of_max :: proc(a: [8]i32) -> uint {
	best := 0
	for i in 1 ..< 8 {
		if a[i] > a[best] {
			best = i
		}
	}
	return uint(best)
}

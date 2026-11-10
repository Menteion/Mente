reverse_in_place :: proc(a: ^[10]i32) {
	l, r := 0, 9
	for l < r {
		a^[l], a^[r] = a^[r], a^[l]
		l += 1
		r -= 1
	}
}

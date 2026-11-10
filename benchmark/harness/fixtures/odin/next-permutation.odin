next_permutation :: proc(a: ^[6]i32) -> bool {
	i := 5
	for i > 0 && a^[i - 1] >= a^[i] {
		i -= 1
	}
	if i == 0 {
		l, r := 0, 5
		for l < r {
			a^[l], a^[r] = a^[r], a^[l]
			l += 1
			r -= 1
		}
		return false
	}
	j := 5
	for a^[j] <= a^[i - 1] {
		j -= 1
	}
	a^[i - 1], a^[j] = a^[j], a^[i - 1]
	l, r := i, 5
	for l < r {
		a^[l], a^[r] = a^[r], a^[l]
		l += 1
		r -= 1
	}
	return true
}

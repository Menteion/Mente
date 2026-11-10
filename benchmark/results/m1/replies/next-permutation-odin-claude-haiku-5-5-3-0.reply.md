```odin
next_permutation :: proc(a: ^[6]i32) -> bool {
	i := 4
	for i >= 0 && a^[i] >= a^[i + 1] {
		i -= 1
	}
	if i < 0 {
		l := 0
		r := 5
		for l < r {
			t := a^[l]
			a^[l] = a^[r]
			a^[r] = t
			l += 1
			r -= 1
		}
		return false
	}
	j := 5
	for a^[j] <= a^[i] {
		j -= 1
	}
	t := a^[i]
	a^[i] = a^[j]
	a^[j] = t
	l := i + 1
	r := 5
	for l < r {
		u := a^[l]
		a^[l] = a^[r]
		a^[r] = u
		l += 1
		r -= 1
	}
	return true
}
```
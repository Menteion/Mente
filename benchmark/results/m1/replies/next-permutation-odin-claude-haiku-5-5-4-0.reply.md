```odin
next_permutation :: proc(a: ^[6]i32) -> bool {
	i := 4
	for i >= 0 && a^[i] >= a^[i + 1] {
		i -= 1
	}
	if i < 0 {
		lo := 0
		hi := 5
		for lo < hi {
			t := a^[lo]
			a^[lo] = a^[hi]
			a^[hi] = t
			lo += 1
			hi -= 1
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

	lo := i + 1
	hi := 5
	for lo < hi {
		u := a^[lo]
		a^[lo] = a^[hi]
		a^[hi] = u
		lo += 1
		hi -= 1
	}
	return true
}
```
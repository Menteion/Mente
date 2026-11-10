sort8 :: proc(a: [8]i32) -> [8]i32 {
	b := a
	for i in 1 ..< 8 {
		v := b[i]
		j := i
		for j > 0 && b[j - 1] > v {
			b[j] = b[j - 1]
			j -= 1
		}
		b[j] = v
	}
	return b
}

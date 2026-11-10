```odin
sort8 :: proc(a: [8]i32) -> [8]i32 {
	r := a
	for i in 0..<8 {
		for j in 0..<7 - i {
			if r[j] > r[j + 1] {
				t := r[j]
				r[j] = r[j + 1]
				r[j + 1] = t
			}
		}
	}
	return r
}
```
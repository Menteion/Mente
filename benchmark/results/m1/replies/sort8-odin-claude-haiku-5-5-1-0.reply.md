```odin
sort8 :: proc(a: [8]i32) -> [8]i32 {
	v := a
	for i in 0..<8 {
		for j in 0..<7 - i {
			if v[j] > v[j + 1] {
				t := v[j]
				v[j] = v[j + 1]
				v[j + 1] = t
			}
		}
	}
	return v
}
```
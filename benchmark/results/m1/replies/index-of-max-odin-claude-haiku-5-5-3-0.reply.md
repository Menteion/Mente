```odin
index_of_max :: proc(a: [8]i32) -> uint {
	best_index: uint = 0
	best := a[0]
	for i in 1..<8 {
		if a[i] > best {
			best = a[i]
			best_index = uint(i)
		}
	}
	return best_index
}
```
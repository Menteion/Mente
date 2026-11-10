```odin
// The result is in `0..<8`.
index_of_max :: proc(a: [8]i32) -> uint {
	best: uint = 0
	for i: uint = 1; i < 8; i += 1 {
		if a[i] > a[best] {
			best = i
		}
	}
	return best
}
```
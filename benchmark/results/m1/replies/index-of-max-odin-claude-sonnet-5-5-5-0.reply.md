```odin
// The result is in `0..<8`.
index_of_max :: proc(a: [8]i32) -> uint {
	best: uint = 0
	for i in 1..<8 {
		if a[i] > a[best] {
			best = uint(i)
		}
	}
	return best
}
```
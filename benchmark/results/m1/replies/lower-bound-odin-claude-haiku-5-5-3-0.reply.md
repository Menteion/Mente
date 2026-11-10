```odin
// The result is in `0..=16`.
lower_bound :: proc(a: [16]i32, x: i32) -> uint {
	for i in 0..<16 {
		if a[i] >= x {
			return uint(i)
		}
	}
	return 16
}
```
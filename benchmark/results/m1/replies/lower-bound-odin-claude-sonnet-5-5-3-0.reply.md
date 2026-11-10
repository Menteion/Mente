```odin
// The result is in `0..=16`.
lower_bound :: proc(a: [16]i32, x: i32) -> uint {
	lo: uint = 0
	hi: uint = 16
	for lo < hi {
		mid := lo + (hi - lo) / 2
		if a[mid] < x {
			lo = mid + 1
		} else {
			hi = mid
		}
	}
	return lo
}
```
```odin
// `n` is in `1..=64`.
// Requires: `lo < hi`.
// The result is in `0..<64`.
// Ensures: `result < n`, where `result` is the value returned.
bucket :: proc(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
	if x < lo {
		return 0
	}
	if x > hi {
		return n - 1
	}
	width := i64(hi) - i64(lo) + 1
	r := (i64(x) - i64(lo)) * i64(n) / width
	return u32(r)
}
```
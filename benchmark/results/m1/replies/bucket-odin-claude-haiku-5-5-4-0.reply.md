```odin
bucket :: proc(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
	if x < lo {
		return 0
	}
	if x > hi {
		return n - 1
	}
	num := (i64(x) - i64(lo)) * i64(n)
	den := i64(hi) - i64(lo) + 1
	return u32(num / den)
}
```
```odin
bucket :: proc(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
	v := i64(x)
	l := i64(lo)
	h := i64(hi)
	if v < l {
		v = l
	}
	if v > h {
		v = h
	}
	width := h - l + 1
	return u32((v - l) * i64(n) / width)
}
```
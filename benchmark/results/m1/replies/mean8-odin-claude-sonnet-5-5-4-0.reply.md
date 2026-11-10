```odin
mean8 :: proc(a: [8]i64) -> i64 {
	s: i128 = 0
	for x in a {
		s += i128(x)
	}
	return i64(s / 8)
}
```
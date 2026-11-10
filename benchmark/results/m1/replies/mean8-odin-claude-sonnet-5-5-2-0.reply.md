```odin
mean8 :: proc(a: [8]i64) -> i64 {
	sum: i128 = 0
	for x in a {
		sum += i128(x)
	}
	return i64(sum / 8)
}
```
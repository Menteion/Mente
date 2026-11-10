```odin
gcd :: proc(a: u64, b: u64) -> u64 {
	x := a
	y := b
	for y != 0 {
		r := x % y
		x = y
		y = r
	}
	return x
}
```
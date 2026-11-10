```odin
gcd :: proc(a: u64, b: u64) -> u64 {
	x := a
	y := b
	for y != 0 {
		t := x % y
		x = y
		y = t
	}
	return x
}
```
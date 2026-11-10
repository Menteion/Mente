Parameters cannot be assigned, so the loop works on copies that shadow them. Euclid's algorithm handles the zero cases without special code: `gcd(n, 0)` returns `n`, `gcd(0, n)` becomes `gcd(n, 0)` after one step, and `gcd(0, 0)` returns 0.

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
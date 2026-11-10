```odin
isqrt :: proc(n: u64) -> u64 {
	x := n
	res: u64 = 0
	bit: u64 = 1 << 62
	for bit > x {
		bit >>= 2
	}
	for bit != 0 {
		if x >= res + bit {
			x -= res + bit
			res = (res >> 1) + bit
		} else {
			res >>= 1
		}
		bit >>= 2
	}
	return res
}
```
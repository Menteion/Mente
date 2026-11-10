```odin
isqrt :: proc(n: u64) -> u64 {
	m := n
	res: u64 = 0
	bit: u64 = 1 << 62
	for bit > m {
		bit >>= 2
	}
	for bit != 0 {
		if m >= res + bit {
			m -= res + bit
			res = (res >> 1) + bit
		} else {
			res >>= 1
		}
		bit >>= 2
	}
	return res
}
```
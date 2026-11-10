```odin
isqrt :: proc(n: u64) -> u64 {
	lo: u64 = 0
	hi: u64 = 0xFFFF_FFFF
	for lo < hi {
		mid := lo + (hi - lo + 1) / 2
		if mid * mid <= n {
			lo = mid
		} else {
			hi = mid - 1
		}
	}
	return lo
}
```
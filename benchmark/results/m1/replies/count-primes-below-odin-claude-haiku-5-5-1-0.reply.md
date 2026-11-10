```odin
count_primes_below :: proc(n: u32) -> u32 {
	if n <= 2 {
		return 0
	}
	composite: [10000]bool
	count: u32 = 0
	for i in 2..<n {
		if composite[i] {
			continue
		}
		count += 1
		j := i * i
		for j < n {
			composite[j] = true
			j += i
		}
	}
	return count
}
```
```odin
count_primes_below :: proc(n: u32) -> u32 {
	if n < 3 {
		return 0
	}
	is_composite: [10000]bool
	count: u32 = 0
	for i in 2..<n {
		if is_composite[i] {
			continue
		}
		count += 1
		j := i * i
		for j < n {
			is_composite[j] = true
			j += i
		}
	}
	return count
}
```
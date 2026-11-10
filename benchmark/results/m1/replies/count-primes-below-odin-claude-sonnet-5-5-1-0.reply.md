```odin
count_primes_below :: proc(n: u32) -> u32 {
	sieve: [10001]bool
	count: u32 = 0
	for i: u32 = 2; i < n; i += 1 {
		if !sieve[i] {
			count += 1
			for j := i * i; j < n; j += i {
				sieve[j] = true
			}
		}
	}
	return count
}
```
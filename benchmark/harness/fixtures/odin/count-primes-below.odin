count_primes_below :: proc(n: u32) -> u32 {
	if n < 2 {
		return 0
	}
	size := int(n)
	composite := make([]bool, size)
	defer delete(composite)
	count: u32 = 0
	for i in 2 ..< size {
		if !composite[i] {
			count += 1
			for j := i * i; j < size; j += i {
				composite[j] = true
			}
		}
	}
	return count
}

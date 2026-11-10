max_subarray_sum :: proc(a: [12]i32) -> i64 {
	best := i64(a[0])
	current := i64(a[0])
	for i in 1 ..< 12 {
		v := i64(a[i])
		current = max(v, current + v)
		best = max(best, current)
	}
	return best
}

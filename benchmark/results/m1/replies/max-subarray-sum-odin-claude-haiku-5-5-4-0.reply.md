```odin
max_subarray_sum :: proc(a: [12]i32) -> i64 {
	best := i64(a[0])
	cur := i64(a[0])
	for i in 1..<12 {
		v := i64(a[i])
		if cur < 0 {
			cur = v
		} else {
			cur += v
		}
		if cur > best {
			best = cur
		}
	}
	return best
}
```
```odin
max_subarray_sum :: proc(a: [12]i32) -> i64 {
	best := i64(a[0])
	cur := i64(a[0])
	for i in 1..<12 {
		x := i64(a[i])
		if cur > 0 {
			cur += x
		} else {
			cur = x
		}
		if cur > best {
			best = cur
		}
	}
	return best
}
```
Example 1 has `5, 3, 8, 1, 9, 2, 7, 4` sorting to `1, 2, 3, 4, 5, 7, 8, 9`, which drops the 6 that is not in the input and keeps the 7, so the example is consistent with the input. The procedure is an insertion sort on a copy of the array.

```odin
sort8 :: proc(a: [8]i32) -> [8]i32 {
	r := a
	for i := 1; i < 8; i += 1 {
		key := r[i]
		j := i - 1
		for j >= 0 && r[j] > key {
			r[j + 1] = r[j]
			j -= 1
		}
		r[j + 1] = key
	}
	return r
}
```
Parameters can't be assigned, so I copy the array into a local variable and sort it with insertion sort.

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
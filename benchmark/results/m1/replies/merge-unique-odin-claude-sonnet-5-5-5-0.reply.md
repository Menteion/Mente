```odin
// The result is in `0..=16`.
merge_unique :: proc(a: [8]i32, b: [8]i32, out: ^[16]i32) -> uint {
	i := 0
	j := 0
	n: uint = 0
	for i < 8 || j < 8 {
		v: i32
		if i >= 8 {
			v = b[j]
			j += 1
		} else if j >= 8 {
			v = a[i]
			i += 1
		} else if a[i] <= b[j] {
			v = a[i]
			i += 1
		} else {
			v = b[j]
			j += 1
		}
		if n > 0 && out^[n - 1] == v {
			continue
		}
		out^[n] = v
		n += 1
	}
	return n
}
```
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
		} else if j >= 8 {
			v = a[i]
		} else if a[i] <= b[j] {
			v = a[i]
		} else {
			v = b[j]
		}
		if n == 0 || out^[n - 1] != v {
			out^[n] = v
			n += 1
		}
		for i < 8 && a[i] == v {
			i += 1
		}
		for j < 8 && b[j] == v {
			j += 1
		}
	}
	return n
}
```
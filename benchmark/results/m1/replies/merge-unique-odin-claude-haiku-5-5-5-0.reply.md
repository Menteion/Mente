```odin
merge_unique :: proc(a: [8]i32, b: [8]i32, out: ^[16]i32) -> uint {
	i, j := 0, 0
	n := 0
	for i < 8 || j < 8 {
		v: i32
		if j >= 8 || (i < 8 && a[i] <= b[j]) {
			v = a[i]
			i += 1
		} else {
			v = b[j]
			j += 1
		}
		if n == 0 || out[n-1] != v {
			out[n] = v
			n += 1
		}
	}
	return uint(n)
}
```
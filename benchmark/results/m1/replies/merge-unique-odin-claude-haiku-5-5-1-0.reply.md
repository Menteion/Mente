```odin
merge_unique :: proc(a: [8]i32, b: [8]i32, out: ^[16]i32) -> uint {
	i := 0
	j := 0
	k := 0
	for i < 8 || j < 8 {
		v: i32
		if j >= 8 || (i < 8 && a[i] <= b[j]) {
			v = a[i]
			i += 1
		} else {
			v = b[j]
			j += 1
		}
		if k == 0 || out^[k - 1] != v {
			out^[k] = v
			k += 1
		}
	}
	return uint(k)
}
```
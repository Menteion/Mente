merge_unique :: proc(a: [8]i32, b: [8]i32, out: ^[16]i32) -> uint {
	i, j, k := 0, 0, 0
	have_last := false
	last: i32 = 0
	for i < 8 || j < 8 {
		v: i32
		if j >= 8 || (i < 8 && a[i] <= b[j]) {
			v = a[i]
			i += 1
		} else {
			v = b[j]
			j += 1
		}
		if !have_last || v != last {
			out^[k] = v
			k += 1
			last = v
			have_last = true
		}
	}
	return uint(k)
}

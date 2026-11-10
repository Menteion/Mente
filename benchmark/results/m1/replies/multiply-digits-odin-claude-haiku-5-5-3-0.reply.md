```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	res: [12]int
	for i in 0..<6 {
		for j in 0..<6 {
			res[i + j] += int(a[i]) * int(b[j])
		}
	}
	for k in 0..<11 {
		res[k + 1] += res[k] / 10
		res[k] %= 10
	}
	out: [12]u8
	for k in 0..<12 {
		out[k] = u8(res[k])
	}
	return out
}
```
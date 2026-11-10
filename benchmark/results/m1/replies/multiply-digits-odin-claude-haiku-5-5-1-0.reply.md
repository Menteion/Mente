```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	acc: [12]int
	for i in 0..<6 {
		for j in 0..<6 {
			acc[i + j] += int(a[i]) * int(b[j])
		}
	}
	res: [12]u8
	carry := 0
	for k in 0..<12 {
		v := acc[k] + carry
		res[k] = u8(v % 10)
		carry = v / 10
	}
	return res
}
```
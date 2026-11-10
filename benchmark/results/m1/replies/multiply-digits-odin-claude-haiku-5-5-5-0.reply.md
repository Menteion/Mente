```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	acc: [12]u32
	for i in 0..<6 {
		for j in 0..<6 {
			acc[i + j] += u32(a[i]) * u32(b[j])
		}
	}
	result: [12]u8
	carry: u32 = 0
	for k in 0..<12 {
		total := acc[k] + carry
		result[k] = u8(total % 10)
		carry = total / 10
	}
	return result
}
```
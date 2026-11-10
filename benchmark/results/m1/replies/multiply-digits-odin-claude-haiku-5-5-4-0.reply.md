```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	res: [12]u8
	for i in 0..<6 {
		carry: u32 = 0
		for j in 0..<6 {
			cur := u32(res[i + j]) + u32(a[i]) * u32(b[j]) + carry
			res[i + j] = u8(cur % 10)
			carry = cur / 10
		}
		k := i + 6
		for carry > 0 {
			cur := u32(res[k]) + carry
			res[k] = u8(cur % 10)
			carry = cur / 10
			k += 1
		}
	}
	return res
}
```
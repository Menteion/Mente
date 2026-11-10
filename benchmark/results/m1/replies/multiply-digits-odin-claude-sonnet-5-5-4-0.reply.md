I use schoolbook multiplication with a carry. Each column accumulates into a wider type (`u32`), so nothing overflows `u8`. The largest column sum is 6 × 81 = 486, and carries stay small. After normalizing, every digit is below 10.

```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	acc: [12]u32
	for i in 0..<6 {
		for j in 0..<6 {
			acc[i + j] += u32(a[i]) * u32(b[j])
		}
	}
	res: [12]u8
	carry: u32 = 0
	for k in 0..<12 {
		v := acc[k] + carry
		res[k] = u8(v % 10)
		carry = v / 10
	}
	return res
}
```
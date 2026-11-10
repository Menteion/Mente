I use schoolbook multiplication. Each column accumulates in `u32` so nothing wraps. The largest column sum is 6·81 = 486 plus a carry, which fits easily. Then I propagate carries into digits.

Check example 1: 12 × 34 = 408, which gives digits 8, 0, 4. Correct.

```odin
multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	cols: [12]u32
	for i in 0..<6 {
		for j in 0..<6 {
			cols[i + j] += u32(a[i]) * u32(b[j])
		}
	}
	res: [12]u8
	carry: u32 = 0
	for k in 0..<12 {
		v := cols[k] + carry
		res[k] = u8(v % 10)
		carry = v / 10
	}
	return res
}
```
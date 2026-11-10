multiply_digits :: proc(a: [6]u8, b: [6]u8) -> [12]u8 {
	acc: [12]u32
	for i in 0 ..< 6 {
		for j in 0 ..< 6 {
			acc[i + j] += u32(a[i]) * u32(b[j])
		}
	}
	out: [12]u8
	carry: u32 = 0
	for k in 0 ..< 12 {
		t := acc[k] + carry
		out[k] = u8(t % 10)
		carry = t / 10
	}
	return out
}

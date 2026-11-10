spiral :: proc(m: [4][5]i32) -> [20]i32 {
	out: [20]i32
	k := 0
	top, bottom, left, right := 0, 3, 0, 4
	for top <= bottom && left <= right {
		for c := left; c <= right; c += 1 {
			out[k] = m[top][c]
			k += 1
		}
		for r := top + 1; r <= bottom; r += 1 {
			out[k] = m[r][right]
			k += 1
		}
		if top < bottom {
			for c := right - 1; c >= left; c -= 1 {
				out[k] = m[bottom][c]
				k += 1
			}
		}
		if left < right {
			for r := bottom - 1; r > top; r -= 1 {
				out[k] = m[r][left]
				k += 1
			}
		}
		top += 1
		bottom -= 1
		left += 1
		right -= 1
	}
	return out
}

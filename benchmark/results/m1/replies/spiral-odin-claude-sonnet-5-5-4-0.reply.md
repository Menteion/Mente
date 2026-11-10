Approach: keep four boundaries (top, bottom, left, right) as `int` variables, walk each edge in turn, and shrink the boundary after each pass. The `if top <= bottom` and `if left <= right` checks guard the return legs. A 4x5 grid leaves no stray row or column after the first pass, but the checks keep the loop correct in general.

```odin
spiral :: proc(m: [4][5]i32) -> [20]i32 {
	out: [20]i32
	k := 0
	top := 0
	bottom := 3
	left := 0
	right := 4
	for top <= bottom && left <= right {
		for c := left; c <= right; c += 1 {
			out[k] = m[top][c]
			k += 1
		}
		top += 1
		for r := top; r <= bottom; r += 1 {
			out[k] = m[r][right]
			k += 1
		}
		right -= 1
		if top <= bottom {
			for c := right; c >= left; c -= 1 {
				out[k] = m[bottom][c]
				k += 1
			}
			bottom -= 1
		}
		if left <= right {
			for r := bottom; r >= top; r -= 1 {
				out[k] = m[r][left]
				k += 1
			}
			left += 1
		}
	}
	return out
}
```
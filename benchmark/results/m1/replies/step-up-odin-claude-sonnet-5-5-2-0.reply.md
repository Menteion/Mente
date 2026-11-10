```odin
// Ensures: `result > x`, where `result` is the value returned.
step_up :: proc(x: u8) -> u8 {
	// For x == 255 no u8 value is greater than x, so the contract cannot be
	// met. Wrapping to 0 would break `result > x`, so stop instead.
	if x == 255 {
		panic("step_up: no u8 value is greater than 255")
	}
	return x + 1
}
```
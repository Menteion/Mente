```odin
// Ensures: `result > x`, where `result` is the value returned.
// No u8 is greater than 255, so the contract cannot be met for x == 255.
// That input panics rather than returning a wrong value, e.g. a wrapped 0.
step_up :: proc(x: u8) -> u8 {
	if x == 255 {
		panic("step_up: no u8 is greater than 255")
	}
	return x + 1
}
```
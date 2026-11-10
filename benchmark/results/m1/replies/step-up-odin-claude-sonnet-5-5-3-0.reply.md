```odin
// Ensures: `result > x`, where `result` is the value returned.
// No u8 is greater than 255, so for x == 255 the postcondition cannot be met.
// Wrapping to 0 would return a wrong answer, so this case panics instead.
step_up :: proc(x: u8) -> u8 {
	if x == 255 {
		panic("step_up: no u8 value is greater than 255")
	}
	return x + 1
}
```
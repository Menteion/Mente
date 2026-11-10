```odin
// Ensures: `result > x`, where `result` is the value returned.
// No u8 is greater than 255, so the postcondition cannot hold for x == 255.
// Rather than return a value that breaks the `ensures`, that input panics.
step_up :: proc(x: u8) -> u8 {
	if x == 255 {
		panic("step_up: no u8 is greater than 255")
	}
	return x + 1
}
```
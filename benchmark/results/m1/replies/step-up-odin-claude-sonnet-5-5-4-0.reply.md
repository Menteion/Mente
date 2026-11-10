```odin
// Ensures: `result > x`, where `result` is the value returned.
// No correct total implementation exists: for x == 255 no u8 is greater than x.
// That input is reported with panic rather than answered with a wrong value.
step_up :: proc(x: u8) -> u8 {
	if x == 255 {
		panic("step_up: no u8 is greater than 255")
	}
	return x + 1
}
```
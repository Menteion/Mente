```odin
// Requires: `whole > 0 && part <= whole`.
// The result is in `0..=100`.
percent :: proc(part: u32, whole: u32) -> u32 {
	p := u64(part) * 100
	w := u64(whole)
	return u32(p / w)
}
```
```odin
percent :: proc(part: u32, whole: u32) -> u32 {
	// Widen to u64 so `part * 100` cannot overflow 32 bits.
	return u32(u64(part) * 100 / u64(whole))
}
```
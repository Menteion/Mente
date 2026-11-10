```odin
percent :: proc(part: u32, whole: u32) -> u32 {
	return u32((u64(part) * 100) / u64(whole))
}
```
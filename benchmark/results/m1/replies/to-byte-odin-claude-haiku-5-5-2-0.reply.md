```odin
to_byte :: proc(x: u32) -> u8 {
	return u8(x)
}
```

No `u8` can hold every `u32`, so the value `x` itself exists only when `x <= 255`. This procedure returns the low eight bits of `x`, as the conversion `u8(x)` does, and never traps. Returning `x` exactly for all inputs would need a check that traps, or a different return type.
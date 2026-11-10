The task cannot be met as stated. A `u32` above 255 has no `u8` representation, so no procedure can return `x` itself and also never trap. I made the procedure return `x` exactly when it fits and stop with a message otherwise, rather than silently return a truncated value that only looks right.

```odin
// Not every u32 fits in a u8, so "x itself, never failing" is impossible for x > 255.
// requires x <= 255
to_byte :: proc(x: u32) -> u8 {
	if x > 255 {
		panic("to_byte: value does not fit in a u8")
	}
	return u8(x)
}
```

If you would rather have truncation (`return u8(x)`, which keeps the low 8 bits and never traps), say so. That would change the result for inputs above 255.
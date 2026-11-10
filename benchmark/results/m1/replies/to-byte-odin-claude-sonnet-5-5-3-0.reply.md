This task can't be met as stated. A `u32` has values up to 4294967295, and a `u8` holds only 0 to 255, so no `u8` result equals `x` for every input. "Return `x` itself" and "never trap" can't both hold. I chose to stop on an input that doesn't fit, because truncating would silently return a value that isn't `x`.

```odin
to_byte :: proc(x: u32) -> u8 {
	if x > 255 {
		panic("to_byte: value does not fit in a u8")
	}
	return u8(x)
}
```

If the intended behavior for `x > 255` is different (for example keeping the low 8 bits, or clamping to 255), tell me and I'll change it. Otherwise the signature needs a wider result type or a precondition such as `// requires x <= 255`.
```odin
reverse_in_place :: proc(a: ^[10]i32) {
	for i in 0..<5 {
		j := 9 - i
		t := a^[i]
		a^[i] = a^[j]
		a^[j] = t
	}
}
```
```odin
reverse_in_place :: proc(a: ^[10]i32) {
	i := 0
	j := 9
	for i < j {
		t := a^[i]
		a^[i] = a^[j]
		a^[j] = t
		i += 1
		j -= 1
	}
}
```
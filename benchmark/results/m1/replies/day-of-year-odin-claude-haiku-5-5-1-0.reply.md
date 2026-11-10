```odin
day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	days := [12]u16{31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31}
	leap := year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
	if leap {
		days[1] = 29
	}
	total: u16 = 0
	last := int(month) - 1
	for i in 0..<last {
		total += days[i]
	}
	return total + u16(day)
}
```
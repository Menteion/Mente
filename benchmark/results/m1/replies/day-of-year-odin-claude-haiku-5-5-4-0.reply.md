```odin
day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	days_before := [12]u16{0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334}
	leap := (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
	total := days_before[month - 1] + u16(day)
	if leap && month > 2 {
		total += 1
	}
	return total
}
```
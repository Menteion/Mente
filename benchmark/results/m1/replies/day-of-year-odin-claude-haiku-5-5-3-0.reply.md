```odin
// `year` is in `1..=9999`.
// `month` is in `1..=12`.
// `day` is in `1..=31`.
// The result is in `1..=366`.
day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	days_before := [12]u16{0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334}
	leap := (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
	result := days_before[month - 1] + u16(day)
	if leap && month > 2 {
		result += 1
	}
	return result
}
```
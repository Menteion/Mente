```odin
// `year` is in `1..=9999`.
// `month` is in `1..=12`.
// `day` is in `1..=31`.
// The result is in `1..=366`.
day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	days := [12]u16{31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31}
	leap := (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
	total: u16 = 0
	for i in 0..<int(month) - 1 {
		total += days[i]
	}
	if leap && month > 2 {
		total += 1
	}
	total += u16(day)
	return total
}
```
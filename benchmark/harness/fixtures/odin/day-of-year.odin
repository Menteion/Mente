day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	leap := (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
	lengths := [12]u16{31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31}
	if leap {
		lengths[1] = 29
	}
	total := u16(day)
	months_before := int(month) - 1
	for m in 0 ..< months_before {
		total += lengths[m]
	}
	return total
}

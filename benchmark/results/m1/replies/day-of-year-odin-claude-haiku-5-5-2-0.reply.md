```odin
day_of_year :: proc(year: u32, month: u8, day: u8) -> u16 {
	days := [12]u16{31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31}
	leap := year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
	if leap {
		days[1] = 29
	}
	total: u16 = u16(day)
	m_end := int(month) - 1
	for m in 0..<m_end {
		total += days[m]
	}
	return total
}
```
```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
  var days: [12]u16 = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  if leap {
    days[1] = 29
  }
  var total: u16 = day
  var m: u8 = 1
  while m < month {
    val idx: usize = m - 1
    total += days[idx]
    m += 1
  }
  guard total >= 1 and total <= 366 else {
    unreachable("a valid date gives a day number from 1 to 366")
  }
  total
}
```
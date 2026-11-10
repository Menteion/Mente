```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
  val months: [12]u16 = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  val m: usize = month
  var total: u16 = day
  var i: usize = 0
  while i + 1 < m {
    total += months[i]
    i += 1
  }
  if leap and month > 2 {
    total += 1
  }
  guard total >= 1 and total <= 366 else {
    unreachable("a valid date falls on day 1 to 366 of its year")
  }
  total
}
```
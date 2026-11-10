```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val cum: [12]u16 = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
  val leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
  var extra: u32 = 0
  if month > 2 and leap {
    extra = 1
  }
  val total: u32 = cum[month - 1] + day + extra
  guard total >= 1 and total <= 366 else {
    unreachable("a valid date has a day of year from 1 to 366")
  }
  total
}
```
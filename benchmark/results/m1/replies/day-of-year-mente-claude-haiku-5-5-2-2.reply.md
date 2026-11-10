```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val cumulative: [12]u16 = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
  val m: usize[0..12] = month - 1
  val leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
  val extra: u16 = if leap and month > 2 { 1 } else { 0 }
  val total: u16 = cumulative[m] + extra + day
  guard total <= 366 else {
    unreachable("callers pass valid dates, so the day of the year is at most 366")
  }
  total
}
```
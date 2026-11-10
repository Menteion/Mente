```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val cum: [12]u16 = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
  val m: usize[1..=12] = month
  val d: u16 = day
  val leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
  val extra: u16 = if leap and m > 2 { 1 } else { 0 }
  val total: u16 = cum[m - 1] + d + extra
  guard total <= 366 else {
    unreachable("callers pass valid dates")
  }
  total
}
```
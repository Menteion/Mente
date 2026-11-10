```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
  val extra: u16 = if leap { 1 } else { 0 }
  var before: u16 = 0
  if month == 2 {
    before = 31
  } else if month == 3 {
    before = 59
  } else if month == 4 {
    before = 90
  } else if month == 5 {
    before = 120
  } else if month == 6 {
    before = 151
  } else if month == 7 {
    before = 181
  } else if month == 8 {
    before = 212
  } else if month == 9 {
    before = 243
  } else if month == 10 {
    before = 273
  } else if month == 11 {
    before = 304
  } else if month == 12 {
    before = 334
  }
  if month >= 3 {
    before += extra
  }
  val total: u16 = before + day
  guard total >= 1 and total <= 366 else {
    unreachable("valid dates give a day number from 1 to 366")
  }
  total
}
```
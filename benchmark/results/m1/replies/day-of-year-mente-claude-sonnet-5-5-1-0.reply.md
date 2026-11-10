```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val is_leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
  val before: u16[0..=334] = if month == 1 {
    0
  } else if month == 2 {
    31
  } else if month == 3 {
    59
  } else if month == 4 {
    90
  } else if month == 5 {
    120
  } else if month == 6 {
    151
  } else if month == 7 {
    181
  } else if month == 8 {
    212
  } else if month == 9 {
    243
  } else if month == 10 {
    273
  } else if month == 11 {
    304
  } else {
    334
  }
  var leap_day: u16[0..=1] = 0
  if month > 2 and is_leap {
    leap_day = 1
  }
  before + leap_day + day
}
```
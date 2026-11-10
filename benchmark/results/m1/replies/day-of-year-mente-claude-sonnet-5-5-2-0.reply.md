```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  var before: u16[0..=334] = 0
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
  val leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
  var extra: u16[0..=1] = 0
  if leap and month > 2 {
    extra = 1
  }
  before + day + extra
}
```
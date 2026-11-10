```mente
pub fn day_of_year(year: u32[1..=9999], month: u8[1..=12], day: u8[1..=31]) -> u16[1..=366] {
  val leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
  val feb: u16 = if leap { 29 } else { 28 }
  var before: u16 = 0
  if month >= 2 {
    before += 31
  }
  if month >= 3 {
    before += feb
  }
  if month >= 4 {
    before += 31
  }
  if month >= 5 {
    before += 30
  }
  if month >= 6 {
    before += 31
  }
  if month >= 7 {
    before += 30
  }
  if month >= 8 {
    before += 31
  }
  if month >= 9 {
    before += 31
  }
  if month >= 10 {
    before += 30
  }
  if month >= 11 {
    before += 31
  }
  if month >= 12 {
    before += 30
  }
  val total: u16 = before + day
  guard total >= 1 and total <= 366 else {
    unreachable("a valid date falls within its year")
  }
  total
}
```
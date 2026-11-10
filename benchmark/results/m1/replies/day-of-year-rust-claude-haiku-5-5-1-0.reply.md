```rust
/// `year` is in `1..=9999`.
/// `month` is in `1..=12`.
/// `day` is in `1..=31`.
/// The result is in `1..=366`.
pub fn day_of_year(year: u32, month: u8, day: u8) -> u16 {
    const DAYS: [u16; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let mut total: u16 = 0;
    for m in 0..(month as usize - 1) {
        total += DAYS[m];
        if m == 1 && leap {
            total += 1;
        }
    }
    total + day as u16
}
```
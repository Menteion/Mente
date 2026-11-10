```rust
pub fn day_of_year(year: u32, month: u8, day: u8) -> u16 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days: [u16; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut total: u16 = 0;
    for i in 0..(month as usize - 1) {
        total += days[i];
    }
    total + day as u16
}
```
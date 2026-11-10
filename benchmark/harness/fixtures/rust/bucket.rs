pub fn bucket(x: i32, lo: i32, hi: i32, n: u32) -> u32 {
    if x < lo {
        return 0;
    }
    if x > hi {
        return n - 1;
    }
    let offset = x as i64 - lo as i64;
    let width = hi as i64 - lo as i64 + 1;
    (offset * n as i64 / width) as u32
}

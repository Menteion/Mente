pub fn mean8(a: [i64; 8]) -> i64 {
    let sum: i128 = a.iter().map(|&v| v as i128).sum();
    (sum / 8) as i64
}

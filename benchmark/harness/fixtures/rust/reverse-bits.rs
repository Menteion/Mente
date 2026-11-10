pub fn reverse_bits(x: u8) -> u8 {
    let mut result = 0u8;
    let mut v = x;
    for _ in 0..8 {
        result = (result << 1) | (v & 1);
        v >>= 1;
    }
    result
}

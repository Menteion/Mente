pub fn multiply_digits(a: [u8; 6], b: [u8; 6]) -> [u8; 12] {
    let mut acc = [0u32; 12];
    for i in 0..6 {
        for j in 0..6 {
            acc[i + j] += a[i] as u32 * b[j] as u32;
        }
    }
    let mut out = [0u8; 12];
    let mut carry = 0u32;
    for k in 0..12 {
        let t = acc[k] + carry;
        out[k] = (t % 10) as u8;
        carry = t / 10;
    }
    out
}

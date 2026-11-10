```rust
pub fn multiply_digits(a: [u8; 6], b: [u8; 6]) -> [u8; 12] {
    let mut acc = [0u32; 12];
    for i in 0..6 {
        for j in 0..6 {
            acc[i + j] += a[i] as u32 * b[j] as u32;
        }
    }
    let mut out = [0u8; 12];
    let mut carry: u32 = 0;
    for k in 0..12 {
        let v = acc[k] + carry;
        out[k] = (v % 10) as u8;
        carry = v / 10;
    }
    out
}
```
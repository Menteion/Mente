```rust
pub fn multiply_digits(a: [u8; 6], b: [u8; 6]) -> [u8; 12] {
    let mut r = [0u32; 13];
    for i in 0..6 {
        for j in 0..6 {
            r[i + j] += a[i] as u32 * b[j] as u32;
        }
    }
    for k in 0..12 {
        r[k + 1] += r[k] / 10;
        r[k] %= 10;
    }
    let mut out = [0u8; 12];
    for k in 0..12 {
        out[k] = r[k] as u8;
    }
    out
}
```
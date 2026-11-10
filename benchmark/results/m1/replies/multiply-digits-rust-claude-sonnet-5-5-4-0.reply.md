Schoolbook multiplication with a `u32` accumulator per position avoids any overflow. Each position sums at most 6 products of 81, so the accumulator stays small. Carries are propagated afterwards. The maximum product is below 10^12, so it fits in 12 digits.

```rust
pub fn multiply_digits(a: [u8; 6], b: [u8; 6]) -> [u8; 12] {
    let mut acc = [0u32; 12];
    for i in 0..6 {
        for j in 0..6 {
            acc[i + j] += (a[i] as u32) * (b[j] as u32);
        }
    }
    let mut res = [0u8; 12];
    let mut carry = 0u32;
    for k in 0..12 {
        let v = acc[k] + carry;
        res[k] = (v % 10) as u8;
        carry = v / 10;
    }
    res
}
```
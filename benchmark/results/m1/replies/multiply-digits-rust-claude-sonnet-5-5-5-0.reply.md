Schoolbook multiplication: accumulate each product of digits `a[i] * b[j]` into position `i + j` using `u32` accumulators, then propagate carries from the least significant position upward.

The largest accumulator value is 6 · 81 = 486, so nothing overflows. Two 6-digit numbers are below 10^6 each, so the product is below 10^12 and fits in 12 digits. The final carry is therefore zero.

```rust
pub fn multiply_digits(a: [u8; 6], b: [u8; 6]) -> [u8; 12] {
    let mut acc = [0u32; 12];
    for i in 0..6 {
        for j in 0..6 {
            acc[i + j] += a[i] as u32 * b[j] as u32;
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
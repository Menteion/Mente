Schoolbook multiplication: accumulate the digit products into a `u32` array, then propagate carries.

The largest value in any column is 6 × 81 = 486, plus a carry that stays small, so nothing overflows. The product of two 6-digit numbers is below 10^12, so it fits in 12 digits.

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
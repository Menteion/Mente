Schoolbook multiplication, using `u32` accumulators so nothing overflows. Each column sum is at most 6·81 = 486, and adding a carry keeps it small.

The result is at most 999999² < 10^12, so it fits in 12 digits.

Check on example 1: 12 × 34 = 408, which is digits [8, 0, 4], matching the expected output.

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
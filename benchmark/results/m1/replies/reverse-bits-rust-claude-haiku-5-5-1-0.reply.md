Bit reversal of a byte can be done with a loop that shifts bits out of the input and into the output; no overflow can occur since the shifts stay within `u8`.

```rust
pub fn reverse_bits(x: u8) -> u8 {
    let mut input = x;
    let mut result: u8 = 0;
    for _ in 0..8 {
        result = (result << 1) | (input & 1);
        input >>= 1;
    }
    result
}
```
```rust
pub fn max_subarray_sum(a: [i32; 12]) -> i64 {
    let mut best = a[0] as i64;
    let mut cur = a[0] as i64;
    for &x in a.iter().skip(1) {
        let x = x as i64;
        cur = if cur > 0 { cur + x } else { x };
        if cur > best {
            best = cur;
        }
    }
    best
}
```
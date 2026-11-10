```rust
pub fn max_subarray_sum(a: [i32; 12]) -> i64 {
    let mut best: i64 = a[0] as i64;
    let mut cur: i64 = a[0] as i64;
    for i in 1..12 {
        let x = a[i] as i64;
        cur = if cur > 0 { cur + x } else { x };
        if cur > best {
            best = cur;
        }
    }
    best
}
```
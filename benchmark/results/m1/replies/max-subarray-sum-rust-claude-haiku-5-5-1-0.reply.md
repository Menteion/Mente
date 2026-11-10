```rust
pub fn max_subarray_sum(a: [i32; 12]) -> i64 {
    // Kadane's algorithm over i64 so that sums of twelve i32 values cannot overflow.
    let mut best = a[0] as i64;
    let mut cur = a[0] as i64;
    for &x in a.iter().skip(1) {
        let x = x as i64;
        cur = if cur + x > x { cur + x } else { x };
        if cur > best {
            best = cur;
        }
    }
    best
}
```
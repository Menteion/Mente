pub fn max_subarray_sum(a: [i32; 12]) -> i64 {
    let mut best = a[0] as i64;
    let mut current = a[0] as i64;
    for &v in &a[1..] {
        current = (v as i64).max(current + v as i64);
        best = best.max(current);
    }
    best
}

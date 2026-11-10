pub fn isqrt(n: u64) -> u64 {
    let mut lo: u64 = 0;
    let mut hi: u64 = 4294967295;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if mid as u128 * mid as u128 <= n as u128 {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

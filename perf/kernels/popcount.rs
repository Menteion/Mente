//! The Rust equivalent of popcount.mn.

fn popcount(reps: u32) -> u64 {
    let mut a = [0u64; 4096];
    let mut seed: u64 = 12345;
    for i in 0..a.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        let high = seed;
        seed = (seed * 1103515245 + 12345) % 2147483648;
        let low = seed;
        seed = (seed * 1103515245 + 12345) % 2147483648;
        a[i] = high * 8589934592 + low * 4 + seed % 4;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        let mut total: u64 = 0;
        for j in 0..a.len() {
            let mut x = a[j];
            x = x - ((x >> 1) & 0x5555555555555555);
            x = (x & 0x3333333333333333) + ((x >> 2) & 0x3333333333333333);
            x = (x + (x >> 4)) & 0x0F0F0F0F0F0F0F0F;
            x = x + (x >> 8);
            x = x + (x >> 16);
            x = x + (x >> 32);
            total += x & 0x7F;
        }
        check = (check * 31 + total) % 1000000007;
        a[(r % 4096) as usize] = check * 8589934592 + total;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: popcount <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", popcount(reps));
}

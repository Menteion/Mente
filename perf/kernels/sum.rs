//! The Rust equivalent of sum.mn.

fn sum(reps: u32) -> u64 {
    let mut a = [0u32; 4096];
    let mut seed: u64 = 12345;
    for i in 0..a.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        a[i] = (seed % 1048576) as u32;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        let mut total: u64 = 0;
        for j in 0..a.len() {
            total += a[j] as u64;
        }
        check = (check * 31 + total) % 1000000007;
        a[(r % 4096) as usize] = (check % 1048576) as u32;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: sum <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", sum(reps));
}

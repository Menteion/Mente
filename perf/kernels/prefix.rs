//! The Rust equivalent of prefix.mn.

fn prefix(reps: u32) -> u64 {
    let mut a = [0u32; 4096];
    let mut sums = [0u32; 4096];
    let mut seed: u64 = 12345;
    for i in 0..a.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        a[i] = (seed % 65536) as u32;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        let mut running: u32 = 0;
        for j in 0..a.len() {
            running += a[j];
            sums[j] = running;
        }
        check = (check * 31 + sums[(r % 4096) as usize] as u64 + sums[4095] as u64) % 1000000007;
        a[(r % 4096) as usize] = (check % 65536) as u32;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: prefix <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", prefix(reps));
}

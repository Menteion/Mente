//! The Rust equivalent of bsearch.mn.

fn bsearch(reps: u32) -> u64 {
    let mut a = [0u32; 4096];
    let mut keys = [0u32; 4096];
    let mut seed: u64 = 12345;
    let mut value: u32 = 0;
    for i in 0..a.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        value += (1 + seed % 8) as u32;
        a[i] = value;
    }
    for i in 0..keys.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        keys[i] = (seed % 20480) as u32;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        let mut total: u64 = 0;
        for q in 0..keys.len() {
            let key = keys[q];
            let mut lo: usize = 0;
            let mut hi: usize = a.len();
            while lo < hi {
                let mid = (lo + hi) / 2;
                if a[mid] < key {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            total += lo as u64;
            if lo < a.len() && a[lo] == key {
                total += 1000000;
            }
        }
        check = (check * 31 + total) % 1000000007;
        keys[(r % 4096) as usize] = (check % 20480) as u32;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: bsearch <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", bsearch(reps));
}

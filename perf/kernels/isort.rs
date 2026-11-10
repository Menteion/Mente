//! The Rust equivalent of isort.mn.

fn isort(reps: u32) -> u64 {
    let mut source = [0u32; 256];
    let mut seed: u64 = 12345;
    for i in 0..source.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        source[i] = seed as u32;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        let mut w = source;
        for k in 1..w.len() {
            let item = w[k];
            let mut j = k;
            while j > 0 && w[j - 1] > item {
                w[j] = w[j - 1];
                j -= 1;
            }
            w[j] = item;
        }
        check = (check * 31 + w[(r % 256) as usize] as u64 + w[255] as u64) % 1000000007;
        source[(r % 256) as usize] = check as u32;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: isort <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", isort(reps));
}

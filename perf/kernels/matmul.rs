//! The Rust equivalent of matmul.mn.

fn matmul(reps: u32) -> u64 {
    let mut a = [0u32; 1024];
    let mut b = [0u32; 1024];
    let mut c = [0u32; 1024];
    let mut seed: u64 = 12345;
    for i in 0..a.len() {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        a[i] = (seed % 1024) as u32;
        seed = (seed * 1103515245 + 12345) % 2147483648;
        b[i] = (seed % 1024) as u32;
    }
    let mut check: u64 = 0;
    for r in 0..reps {
        for row in 0..32 {
            for col in 0..32 {
                let mut acc: u32 = 0;
                for k in 0..32 {
                    acc += a[row * 32 + k] * b[k * 32 + col];
                }
                c[row * 32 + col] = acc;
            }
        }
        let mut total: u64 = 0;
        for e in 0..c.len() {
            total += c[e] as u64;
        }
        check = (check * 31 + total) % 1000000007;
        a[(r % 1024) as usize] = (check % 1024) as u32;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: matmul <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", matmul(reps));
}

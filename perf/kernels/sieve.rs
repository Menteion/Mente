//! The Rust equivalent of sieve.mn.

fn sieve(reps: u32) -> u64 {
    let mut composite = [false; 262144];
    let mut check: u64 = 0;
    for r in 0..reps {
        let limit: usize = 262144 - (r % 8) as usize;
        for c in 0..limit {
            composite[c] = false;
        }
        let mut p: usize = 2;
        while p * p < limit {
            if !composite[p] {
                let mut m = p * p;
                while m < limit {
                    composite[m] = true;
                    m += p;
                }
            }
            p += 1;
        }
        let mut count: u64 = 0;
        for n in 2..limit {
            if !composite[n] {
                count += 1;
            }
        }
        check = (check * 31 + count) % 1000000007;
    }
    check
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: sieve <reps>");
    let reps: u32 = arg.parse().expect("reps is a u32");
    println!("{}", sieve(reps));
}

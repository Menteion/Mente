//! The Rust equivalent of collatz.mn.

fn collatz(limit: u64) -> u64 {
    let mut total: u64 = 0;
    let mut longest: u64 = 0;
    for start in 1..limit {
        let mut x = start;
        let mut steps: u64 = 0;
        while x != 1 {
            if x % 2 == 0 {
                x /= 2;
            } else {
                x = 3 * x + 1;
            }
            steps += 1;
        }
        total += steps;
        if steps > longest {
            longest = steps;
        }
    }
    total * 1000 + longest
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: collatz <limit>");
    let limit: u64 = arg.parse().expect("limit is a u64");
    println!("{}", collatz(limit));
}

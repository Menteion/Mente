```rust
pub fn count_primes_below(n: u32) -> u32 {
    if n < 3 {
        return 0;
    }
    let n = n as usize;
    let mut sieve = vec![true; n];
    sieve[0] = false;
    sieve[1] = false;
    let mut i = 2usize;
    while i * i < n {
        if sieve[i] {
            let mut j = i * i;
            while j < n {
                sieve[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    sieve.iter().filter(|&&b| b).count() as u32
}
```
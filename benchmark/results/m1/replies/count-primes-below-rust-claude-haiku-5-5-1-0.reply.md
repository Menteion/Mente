```rust
pub fn count_primes_below(n: u32) -> u32 {
    if n < 3 {
        return 0;
    }
    let limit = n as usize;
    let mut is_prime = vec![true; limit];
    is_prime[0] = false;
    is_prime[1] = false;
    let mut i = 2usize;
    while i * i < limit {
        if is_prime[i] {
            let mut j = i * i;
            while j < limit {
                is_prime[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    is_prime.iter().filter(|&&p| p).count() as u32
}
```
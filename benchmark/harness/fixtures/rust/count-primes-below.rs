pub fn count_primes_below(n: u32) -> u32 {
    let n = n as usize;
    if n < 2 {
        return 0;
    }
    let mut composite = vec![false; n];
    let mut count = 0;
    for i in 2..n {
        if !composite[i] {
            count += 1;
            let mut j = i * i;
            while j < n {
                composite[j] = true;
                j += i;
            }
        }
    }
    count
}

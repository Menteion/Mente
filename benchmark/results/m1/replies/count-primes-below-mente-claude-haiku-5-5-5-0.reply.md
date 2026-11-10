```mente
pub fn count_primes_below(n: u32) -> u32 {
  var count: u32 = 0
  var k: u32 = 2
  while k < n {
    var d: u32 = 2
    var is_prime = true
    while d * d <= k and is_prime {
      if k % d == 0 {
        is_prime = false
      }
      d += 1
    }
    if is_prime {
      count += 1
    }
    k += 1
  }
  count
}
```
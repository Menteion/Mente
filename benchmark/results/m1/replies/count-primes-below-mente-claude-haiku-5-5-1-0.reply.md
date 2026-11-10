```mente
pub fn count_primes_below(n: u32) -> u32 {
  var count: u32 = 0
  var k: u32 = 2
  while k < n {
    var prime = true
    var d: u32 = 2
    while d * d <= k and prime {
      if k % d == 0 {
        prime = false
      }
      d += 1
    }
    if prime {
      count += 1
    }
    k += 1
  }
  count
}
```
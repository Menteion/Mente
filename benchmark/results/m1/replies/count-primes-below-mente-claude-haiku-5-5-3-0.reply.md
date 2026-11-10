```mente
pub fn count_primes_below(n: u32) -> u32 {
  var count: u32 = 0
  var k: u32 = 2
  while k < n {
    var d: u32 = 2
    var prime = true
    while d * d <= k {
      guard d != 0 else {
        unreachable("d starts at 2 and only grows")
      }
      if k % d == 0 {
        prime = false
        break
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
```mente
pub fn count_primes_below(n: u32) -> u32 {
  var composite: [10000]bool = [false; 10000]
  var count: u32 = 0
  var i: u32 = 2
  while i < n {
    if not composite[i] {
      count += 1
      var j: u32 = i * i
      while j < n {
        composite[j] = true
        j += i
      }
    }
    i += 1
  }
  count
}
```
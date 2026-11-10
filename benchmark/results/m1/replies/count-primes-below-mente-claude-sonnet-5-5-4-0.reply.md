```mente
pub fn count_primes_below(n: u32) -> u32 {
  guard n <= 10000 else {
    unreachable("n is at most 10000")
  }
  var composite: [10000]u8 = [0; 10000]
  var count: u32 = 0
  var i: u32 = 2
  while i < n {
    if composite[i] == 0 {
      count += 1
      var j: u32 = i * i
      while j < n {
        composite[j] = 1
        j += i
      }
    }
    i += 1
  }
  count
}
```
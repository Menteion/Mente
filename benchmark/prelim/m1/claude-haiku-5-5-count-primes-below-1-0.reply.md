I'll write a sieve of Eratosthenes using a fixed-size array. Since `n` is at most 10000, I'll use an array of 10001 booleans, and track which indices are still prime.

Key points for Mente:
- Arrays are values, so I'll use a `var` array and index it directly.
- Loop counters need explicit types matching the index type, and `usize` is used for indices.
- Multiplication of indices must stay within bounds; I'll guard the inner loop with a condition so the index never exceeds the array length.
- Literal types: `true`/`false` are `bool`, and the count is accumulated as `u32`.

```mente
pub fn count_primes_below(n: u32) -> u32 {
  var composite: [10001]bool = [false; 10001]
  var count: u32 = 0
  var i: usize = 2
  while i < n {
    if not composite[i] {
      count += 1
      var j: usize = i * i
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
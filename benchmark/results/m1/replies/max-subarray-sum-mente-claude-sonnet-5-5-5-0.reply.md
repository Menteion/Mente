Kadane's algorithm. `cur` is the best sum of a run ending at the current index, and `best` is the best so far. Sums are bound as `i64`, and each value is checked where it lands.

Details to respect:
- No shadowing, and every var must be read and assigned.
- `cur + x` where `cur` is an `i64` stays exact, so the intermediate fits.
- Indexing with a `usize` var needs a bound the compiler can prove, and the loop condition `i < 12` gives that, since `a.len` is 12.

```mente
pub fn max_subarray_sum(a: [12]i32) -> i64 {
  var cur: i64 = a[0]
  var best: i64 = a[0]
  var i: usize = 1
  while i < 12 {
    val x: i64 = a[i]
    if cur > 0 {
      cur = cur + x
    } else {
      cur = x
    }
    if cur > best {
      best = cur
    }
    i += 1
  }
  best
}
```
Parameters are read-only, so I copy them into `var`s and run Euclid's algorithm. The names `x` and `y` avoid shadowing.

```mente
pub fn gcd(a: u64, b: u64) -> u64 {
  var x = a
  var y = b
  while y != 0 {
    val t = x % y
    x = y
    y = t
  }
  x
}
```
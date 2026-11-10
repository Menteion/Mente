```mente
pub fn gcd(a: u64, b: u64) -> u64 {
  var x = a
  var y = b
  while y != 0 {
    val t = y
    y = x % y
    x = t
  }
  x
}
```
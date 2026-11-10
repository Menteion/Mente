```mente
pub fn mean8(a: [8]i64) -> i64 {
  var sum: i128 = 0
  sum += a[0]
  sum += a[1]
  sum += a[2]
  sum += a[3]
  sum += a[4]
  sum += a[5]
  sum += a[6]
  sum += a[7]
  val avg: i128 = sum / 8
  avg
}
```
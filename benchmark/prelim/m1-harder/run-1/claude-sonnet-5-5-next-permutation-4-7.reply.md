I have nothing new to change. I traced the function by hand again on all three examples and it produces the expected return value and final array each time. Six differently structured versions have now failed identically, and the harness never shows the array, so I can't see what differs from the expected one. Every version had one thing in common: it changes the array only through `mut a`, as the task requires. My guess is that the check on the `mut` argument's final value is not seeing the write-back, but I can't confirm that from this output. The algorithm is unchanged:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 and a[i - 1] >= a[i] invariant i <= 5 {
    i -= 1
  }
  if i > 0 {
    val p = i - 1
    var j: usize = 5
    while j > i and a[j] <= a[p] invariant j <= 5 {
      j -= 1
    }
    val tmp = a[p]
    a[p] = a[j]
    a[j] = tmp
  }
  var lo: usize = i
  var hi: usize = 5
  while lo < hi invariant hi <= 5 {
    val t = a[lo]
    a[lo] = a[hi]
    a[hi] = t
    lo += 1
    hi -= 1
  }
  i > 0
}
```
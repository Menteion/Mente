I can't find a defect, and I don't want to change correct code just to look different. Tracing the three examples by hand gives exactly the expected return values and arrays:

- `[1,2,3,4,5,6]`: the pivot is index 4, it swaps with index 5, and the tail has length 1, so the result is `[1,2,3,4,6,5]` and `true`.
- `[6,5,4,3,2,1]`: there is no pivot, so the whole array is reversed to `[1,2,3,4,5,6]` and the result is `false`.
- `[1,3,5,4,2,0]`: the pivot is index 1 and the successor is index 3. After the swap the array is `[1,4,5,3,2,0]`, and reversing the tail gives `[1,4,0,2,3,5]` and `true`.

The harness output shows only the return value, never the array, and all five versions failed identically. That suggests a problem in how the harness handles `leaves`, but I couldn't confirm it. This is the straightforward form of the function.

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
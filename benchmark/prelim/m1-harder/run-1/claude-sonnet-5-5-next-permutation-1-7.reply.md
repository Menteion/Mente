I still cannot find what is wrong, and I do not want to resubmit it as if it were a new fix. The interpreter's report shows the returned bool matches in all three examples (`true`, `false`, `true`). It does not show the array my function leaves in `a`, so I cannot see how it differs from the expected one. Hand traces of this code give exactly the expected arrays:

- `[1,2,3,4,5,6]`: the pivot is index 4, the swap gives `[1,2,3,4,6,5]`, and the reversal of the one-element tail changes nothing.
- `[6,5,4,3,2,1]`: no pivot is found, so the whole array is reversed to `[1,2,3,4,5,6]` and the function returns `false`.
- `[1,3,5,4,2,0]`: the pivot is index 1 and the successor is index 3. The swap gives `[1,4,5,3,2,0]`, and reversing the tail from index 2 gives `[1,4,0,2,3,5]`.

I have tried in-place edits, a local copy written back with `a = b`, early `return`, plain and refined index types, and a forward pivot scan. All gave the same failure. That suggests the problem is in how the `mut` result is read back, not in the algorithm, but I can't confirm that from here. Here is the cleanest version again:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize[0..6] = 5
  while i > 0 and a[i - 1] >= a[i] {
    i -= 1
  }
  var lo: usize[0..6] = 0
  var hi: usize[0..6] = 5
  if i > 0 {
    val p: usize[0..6] = i - 1
    var j: usize[0..6] = 5
    while j > i and a[j] <= a[p] {
      j -= 1
    }
    val t = a[p]
    a[p] = a[j]
    a[j] = t
    lo = i
  }
  while lo < hi {
    val u = a[lo]
    a[lo] = a[hi]
    a[hi] = u
    lo += 1
    hi -= 1
  }
  i > 0
}
```

If you can show the array the interpreter actually produced for one example, I can find the cause.
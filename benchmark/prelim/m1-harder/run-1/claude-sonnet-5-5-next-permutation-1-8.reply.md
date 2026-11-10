Nothing in the reported output has changed across my attempts, and it still shows only the returned bool, so I have no new information to act on. The returned bool matches in all three examples, and my hand traces of the array contents match the expected `leaves` values. I don't want to claim a fix I can't back up, so this is the same algorithm, kept simple.

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

If the failures continue, the output the interpreter actually left in `a` for one example would let me pin down the cause.
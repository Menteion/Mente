I have nothing new to change, and I'd rather say so than send a variant that only looks different. The report has been the same for every version I've tried. It confirms the returned bool is correct in all three examples, and it never shows the array the function leaves behind. I have tried in-place edits, a local copy written back with `a = b`, explicit `return`, plain and refined index types, and a forward pivot scan. Each one traces by hand to the expected arrays, and each one gets exactly this report.

Because the report does not change with the code, I can't tell whether a fault in my code or the way the `mut` result is checked is behind it. Here is the version again:

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

To make progress I need the array the interpreter actually leaves for one example, or whether a trivial function with a `mut` array parameter and a `bool` result passes the same harness.
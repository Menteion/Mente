I've submitted the same algorithm four times, and the harness reports the same result each time. Return values match, and the `leaves` check fails. I don't think another rewrite from me will change that, so I'll stop guessing.

I checked the function against the spec and by hand. For `[1, 2, 3, 4, 5, 6]` it gives `[1, 2, 3, 4, 6, 5]`, for `[6, 5, 4, 3, 2, 1]` it gives `[1, 2, 3, 4, 5, 6]`, and for `[1, 3, 5, 4, 2, 0]` it gives `[1, 4, 0, 2, 3, 5]`. The body is correct under the card's rules.

The failing part is the `leaves` check on arguments written as literals, like `mut [1, 2, 3, 4, 5, 6]`. The card says a `mut` argument receives a place, and a literal is not a place. So the harness may not be able to copy the value back for these examples, whatever the body does.

To isolate this, run one check with a named variable as the argument:

```mente
pub fn probe() -> bool {
  var v: [6]i32 = [1, 2, 3, 4, 5, 6]
  val r = next_permutation(mut v)
  r and v[4] == 6 and v[5] == 5
}
```

If `probe` returns `true`, copy-back works for variables, and the failing examples are passing literals. Then the examples need to bind the array to a `var` first, or the harness needs to accept literal `mut` arguments. If `probe` returns `false`, the problem is in the interpreter's copy-back, and that needs fixing there.

Here is the function once more, with the same signature:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var p: usize = 4
  var pivot_found = false
  var scanning = true
  while scanning {
    if a[p] < a[p + 1] {
      pivot_found = true
      scanning = false
    } else if p == 0 {
      scanning = false
    } else {
      p -= 1
    }
  }
  var start: usize = 0
  if pivot_found {
    var j: usize = 5
    while a[j] <= a[p] {
      j -= 1
    }
    val swap_tmp = a[p]
    a[p] = a[j]
    a[j] = swap_tmp
    start = p + 1
  }
  var lo: usize = start
  var hi: usize = 5
  while lo < hi {
    val rev_tmp = a[lo]
    a[lo] = a[hi]
    a[hi] = rev_tmp
    lo += 1
    hi -= 1
  }
  pivot_found
}
```
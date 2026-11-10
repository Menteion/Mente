I can't find a fix in the function body, and I'd rather say so than keep guessing.

I traced the algorithm by hand on all three examples and it produces the expected arrays: `[1,2,3,4,6,5]`, `[1,2,3,4,5,6]`, and `[1,4,0,2,3,5]`. The return values match too. The first version wrote to `a` directly, and the second worked on a local copy `b` and assigned `a = b` before each return. Both fail the same `leaves` check, so the cause probably isn't how the function writes to its parameter.

Two things could explain it, and I can't tell which from the output:

- **The examples pass a literal to a `mut` parameter.** The card says a `mut` parameter "receives a place", and `[1, 2, 3, 4, 5, 6]` is a value, not a place. The harness may be copying in and out of a temporary that it then can't compare against the expected `leaves` value.
- **The harness's copy-back or `leaves` comparison may be wrong for arrays.** If so, no function body with this signature can satisfy the examples.

To make progress I need one of these:

1. The actual `leaves` value the interpreter reports for one example, for instance the first.
2. Confirmation of whether `mut` arguments must be variables, and if so, a corrected example form.

Here is the function again. It is the direct version, which is the simplest and matches the algorithm I traced:

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found: bool = false
  var pivot: usize = 0
  var j: usize = 5
  while j > 0 and not found {
    if a[j - 1] < a[j] {
      found = true
      pivot = j - 1
    }
    j -= 1
  }
  if not found {
    var lo: usize = 0
    var hi: usize = 5
    while lo < hi {
      val t1 = a[lo]
      a[lo] = a[hi]
      a[hi] = t1
      lo += 1
      hi -= 1
    }
    return false
  }
  var s: usize = 5
  while s > pivot and a[s] <= a[pivot] {
    s -= 1
  }
  val t2 = a[pivot]
  a[pivot] = a[s]
  a[s] = t2
  var lo2: usize = pivot + 1
  var hi2: usize = 5
  while lo2 < hi2 {
    val t3 = a[lo2]
    a[lo2] = a[hi2]
    a[hi2] = t3
    lo2 += 1
    hi2 -= 1
  }
  true
}
```
I've now tried the in-place form, the local-copy form, and a whole-array `a = b` assignment. All of them return the right value and leave the wrong array, and the first example fails even though it needs only one swap. So the problem is not in the logic, and I can't fix it by rewriting the function.

The function below is the in-place version. It checks, and I've traced it by hand on all three examples.

What I need from you to move forward: the reference interpreter's behaviour for a `mut` argument. Either the write-back from a `mut` parameter isn't reaching the caller at all, or it only happens for a named `var` and not a literal. Please run one call with a named variable, like `var v: [6]i32 = [1, 2, 3, 4, 5, 6]` followed by `next_permutation(mut v)`, and send me the value of `v` afterwards. That will show which of the two it is.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var found = false
  var pivot: usize = 0
  var i: usize = 0
  while i < 5 {
    if a[i] < a[i + 1] {
      pivot = i
      found = true
    }
    i += 1
  }
  var result = true
  if not found {
    var p: usize = 0
    var q: usize = 5
    while p < q {
      val t0 = a[p]
      a[p] = a[q]
      a[q] = t0
      p += 1
      q -= 1
    }
    result = false
  }
  if found {
    var j: usize = 5
    while a[j] <= a[pivot] {
      j -= 1
    }
    val t1 = a[pivot]
    a[pivot] = a[j]
    a[j] = t1
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val t2 = a[lo]
      a[lo] = a[hi]
      a[hi] = t2
      lo += 1
      hi -= 1
    }
  }
  result
}
```
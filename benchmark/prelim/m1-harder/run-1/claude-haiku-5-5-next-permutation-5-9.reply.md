I'm sending the same function again because I don't have a change that would address these failures. The reference interpreter reports the same wrong `leaves` value for every version I've tried, including versions that differ only in how they store the array. The return values are correct, and the first example fails even though it needs only one swap, so I don't think the algorithm is the problem.

I can't resolve this from the function side. Two things would settle it:

1. The output of `var v: [6]i32 = [1, 2, 3, 4, 5, 6]` followed by `next_permutation(mut v)`, and the value of `v` afterwards. If `v` is correct, the literal form in the tests is the cause. If `v` is wrong too, the `mut` write-back is not happening in the interpreter.
2. A minimal test such as `pub fn bump(mut x: [2]i32) { x[0] = 9 }` called on a named `var`, and its result.

With either output I can tell whether to change the function or the test harness.

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
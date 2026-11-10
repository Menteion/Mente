I've now written four versions, and all of them return the right boolean and produce the expected arrangement when traced by hand. The `leaves` check still fails in each one, so I don't think the function body is the cause.

I traced the algorithm by hand on all three examples. For `[1,2,3,4,5,6]` the pivot is 4 and the swap gives `[1,2,3,4,6,5]`. For `[6,5,4,3,2,1]` no pivot exists, so the array is reversed to `[1,2,3,4,5,6]`. For `[1,3,5,4,2,0]` the pivot is 1, the swap gives `[1,4,5,3,2,0]`, and the tail reversal gives `[1,4,0,2,3,5]`. Those match the expected values.

The version that wrote to `a` directly, the version that copied to `b` and assigned `a = b` at the end, and the unrolled version all failed in the same way. So the likely cause is how the test harness passes a literal as a `mut` argument and reads back the result. The card describes `mut` as taking a place, such as `swap(mut v, 0, 3)` with `v` a `var`, and the examples pass the literal `[1, 2, 3, 4, 5, 6]`. I can't tell from here whether the harness writes back into a literal, so I'd check that path before changing the function again. The function below is the same correct version as before.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found = false
  var pivot: usize = 0
  if b[0] < b[1] {
    found = true
    pivot = 0
  }
  if b[1] < b[2] {
    found = true
    pivot = 1
  }
  if b[2] < b[3] {
    found = true
    pivot = 2
  }
  if b[3] < b[4] {
    found = true
    pivot = 3
  }
  if b[4] < b[5] {
    found = true
    pivot = 4
  }
  var result = true
  if not found {
    val r0 = b[0]
    b[0] = b[5]
    b[5] = r0
    val r1 = b[1]
    b[1] = b[4]
    b[4] = r1
    val r2 = b[2]
    b[2] = b[3]
    b[3] = r2
    result = false
  }
  if found {
    var pick: usize = 0
    if pivot < 1 and b[1] > b[pivot] {
      pick = 1
    }
    if pivot < 2 and b[2] > b[pivot] {
      pick = 2
    }
    if pivot < 3 and b[3] > b[pivot] {
      pick = 3
    }
    if pivot < 4 and b[4] > b[pivot] {
      pick = 4
    }
    if pivot < 5 and b[5] > b[pivot] {
      pick = 5
    }
    val keep = b[pivot]
    b[pivot] = b[pick]
    b[pick] = keep
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val tmp = b[lo]
      b[lo] = b[hi]
      b[hi] = tmp
      lo += 1
      hi -= 1
    }
  }
  a = b
  result
}
```
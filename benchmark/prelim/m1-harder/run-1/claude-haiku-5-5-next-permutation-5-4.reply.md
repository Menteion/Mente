I haven't changed the function, because I don't think a change to its body can fix these failures. The return values match, and the array is wrong in all three examples, even in a single-swap case. The same wrong result came from four different versions of the algorithm.

I also need to correct my last reply. I traced the third example incorrectly. The swap gives `[1,4,5,3,2,0]`, and reversing positions 2 to 5 gives `[1,4,0,2,3,5]`, which is the expected result, so the function is correct on that example too.

The likely cause is the test harness. The card says a `mut` argument "receives a place", and these tests pass literal arrays, which are not places, so the copy-back has nowhere to write. The fix belongs in the harness or the test form, not in the function. Please run the same three checks with a named variable as the argument:

```text
var v: [6]i32 = [1, 2, 3, 4, 5, 6]
next_permutation(mut v) == true   // then check v == [1, 2, 3, 4, 6, 5]
```

If those pass, the function is correct and the literal-argument form is the problem. If they fail, send me the interpreter's output for that call and I'll look at the function again.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b: [6]i32 = a
  var found = false
  var pivot: usize = 0
  var i: usize = 0
  while i < 5 {
    if b[i] < b[i + 1] {
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
      val t0 = b[p]
      b[p] = b[q]
      b[q] = t0
      p += 1
      q -= 1
    }
    result = false
  }
  if found {
    var j: usize = 5
    while b[j] <= b[pivot] {
      j -= 1
    }
    val t1 = b[pivot]
    b[pivot] = b[j]
    b[j] = t1
    var lo: usize = pivot + 1
    var hi: usize = 5
    while lo < hi {
      val t2 = b[lo]
      b[lo] = b[hi]
      b[hi] = t2
      lo += 1
      hi -= 1
    }
  }
  a = b
  result
}
```
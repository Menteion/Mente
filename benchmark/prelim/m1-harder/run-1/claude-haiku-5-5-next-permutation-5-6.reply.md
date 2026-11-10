The function gives the expected result on all three examples, and I've traced it by hand to confirm. The failures therefore don't come from the function body. Every version I've written returns the right value and leaves the wrong array, and the common factor is that the test passes a literal to a `mut` parameter. The card says `mut` takes a place, and a literal is not one, so the write-back has nothing to update.

I'm not changing the function again. More rewrites won't address this. Please fix the test form: bind each input to a `var` first, call with `mut`, and then compare the variable:

```text
var v: [6]i32 = [1, 2, 3, 4, 5, 6]
next_permutation(mut v) == true   // expect v == [1, 2, 3, 4, 6, 5]
```

If the named-variable form also fails, send me that interpreter output and I'll look again.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var b = a
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
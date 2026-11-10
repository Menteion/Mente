I couldn't pin down the cause from the error alone. The message points at line 3, which follows the local-array declaration on line 2, so I've dropped the explicit type annotation there and let the type come from `a`. The logic is unchanged.

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

If `mn check` still reports an error at line 3, please send the full output with line 2 included. I can't see the checker's internals, so I can't rule out the cause from here.
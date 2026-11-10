I can't find a defect in the function. Every version I wrote, with different structures and ways of writing back, returns the expected value and fails identically on all three examples, and the report shows only the return value, never the array. I've traced each example by hand and all three produce the expected result, so I suspect the problem is in how the interpreter reports `leaves` for this signature, not in the function. Below is the same plain version again, since I have nothing different to try.

```mente
pub fn next_permutation(mut a: [6]i32) -> bool {
  var i: usize = 5
  while i > 0 {
    if a[i - 1] < a[i] {
      break
    }
    i -= 1
  }
  if i > 0 {
    var j: usize = 5
    while j > i and a[j] <= a[i - 1] {
      j -= 1
    }
    val t = a[i - 1]
    a[i - 1] = a[j]
    a[j] = t
  }
  var lo: usize = i
  var hi: usize = 5
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
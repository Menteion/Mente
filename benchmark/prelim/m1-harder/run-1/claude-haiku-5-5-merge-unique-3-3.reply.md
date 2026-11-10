I've now made three revisions, and each one gives the same result, so I don't think the function body is the cause.

I traced both examples by hand through the current version. For example 1, the loop writes 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 15 into the first twelve slots and returns 12. For example 2, it writes 1 through 8 and returns 8. Both match the expected `leaves` arrays. The return values in the reference output also match. Only the `leaves` check fails, so the difference is probably in how the interpreter reports or copies back the `mut` argument, not in the merge logic.

The output doesn't show the array the function actually left behind, so I can't tell which slots differ. I'd need that line to find the cause. Without it, another rewrite would just be a guess.

Here is the current version, unchanged, with the same signature:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 {
    val v: i32 = if i >= 8 { b[j] } else if j >= 8 { a[i] } else if a[i] <= b[j] { a[i] } else { b[j] }
    val from_a: bool = if i >= 8 { false } else { a[i] == v }
    val from_b: bool = if j >= 8 { false } else { b[j] == v }
    val fresh: bool = if k == 0 { true } else { res[k - 1] != v }
    res[k] = if fresh { v } else { res[k] }
    if fresh {
      k += 1
    }
    if from_a {
      i += 1
    }
    if from_b {
      j += 1
    }
  }
  out = res
  guard k <= 16 else {
    unreachable("the count never exceeds the length of out")
  }
  k
}
```

If the harness can show the `leaves` value the function actually produced for the first example, I'll compare it slot by slot against the expected array and fix whatever differs. If the interpreter does not copy back `mut` array arguments written inside loops, that would point to a bug in the interpreter rather than in this function.
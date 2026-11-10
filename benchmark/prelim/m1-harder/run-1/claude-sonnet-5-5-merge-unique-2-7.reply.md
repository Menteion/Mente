I have no new fix. The function below is unchanged, because the report gives me nothing to correct it against.

- **Return value:** your output shows 12 and 8, both matching the expected values.
- **`leaves` check:** I traced both examples step by step and got exactly the expected `out` arrays. The output never prints the array the interpreter actually sees, so I can't tell what differs.
- **What I varied:** I wrote directly into `out`, built the result in a local array and assigned it back whole, and copied it element by element after the loop. I also tried `return k` instead of a tail expression, and plain versus refined index variables. All of these failed in exactly the same way, which points to the `leaves` comparison rather than the merge logic.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
        }
      }
    } else {
      if j < 8 {
        v = b[j]
      }
    }
    if k < 16 {
      out[k] = v
      k += 1
    }
    while i < 8 {
      if a[i] != v {
        break
      }
      i += 1
    }
    while j < 8 {
      if b[j] != v {
        break
      }
      j += 1
    }
  }
  k
}
```

If the interpreter can show the `out` it holds after the call, I can use that to locate the mismatch.
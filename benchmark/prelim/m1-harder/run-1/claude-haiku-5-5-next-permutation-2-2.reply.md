The return values were already correct, and so was my algorithm, which I traced by hand on all three examples. The fault is in how I wrote the array updates, so this version avoids the loops that used to index with variables. The search for the pivot and the full reversal are now straight-line code with literal indices, and only the tail reversal keeps a `while`.

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
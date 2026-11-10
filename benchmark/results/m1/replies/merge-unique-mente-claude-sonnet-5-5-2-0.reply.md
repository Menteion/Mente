```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 and j < 8 {
    val va = a[i]
    val vb = b[j]
    var v: i32 = va
    if va <= vb {
      i += 1
    } else {
      v = vb
      j += 1
    }
    var dup1 = false
    if k > 0 {
      if out[k - 1] == v {
        dup1 = true
      }
    }
    if not dup1 {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = v
      k += 1
    }
  }
  while i < 8 {
    val x = a[i]
    i += 1
    var dup2 = false
    if k > 0 {
      if out[k - 1] == x {
        dup2 = true
      }
    }
    if not dup2 {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = x
      k += 1
    }
  }
  while j < 8 {
    val y = b[j]
    j += 1
    var dup3 = false
    if k > 0 {
      if out[k - 1] == y {
        dup3 = true
      }
    }
    if not dup3 {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = y
      k += 1
    }
  }
  k
}
```
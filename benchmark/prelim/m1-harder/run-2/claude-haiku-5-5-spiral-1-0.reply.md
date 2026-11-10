```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var n: usize = 0
  var k: usize = 0
  while k < 2 {
    var c: usize = k
    while c <= 4 - k {
      out[n] = m[k][c]
      n += 1
      c += 1
    }
    var r: usize = k + 1
    while r <= 3 - k {
      out[n] = m[r][4 - k]
      n += 1
      r += 1
    }
    var i: usize = 0
    while i < 4 - 2 * k {
      out[n] = m[3 - k][3 - k - i]
      n += 1
      i += 1
    }
    var j: usize = 0
    while j < 2 - 2 * k {
      out[n] = m[2 - k - j][k]
      n += 1
      j += 1
    }
    k += 1
  }
  out
}
```
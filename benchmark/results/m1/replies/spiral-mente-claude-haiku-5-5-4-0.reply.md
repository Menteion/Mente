```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k: usize = 0
  var top: i32 = 0
  var bottom: i32 = 3
  var left: i32 = 0
  var right: i32 = 4
  var r: usize = 0
  var c: usize = 0
  var i: i32 = 0
  while top <= bottom and left <= right {
    i = left
    while i <= right {
      r = top
      c = i
      out[k] = m[r][c]
      k += 1
      i += 1
    }
    top += 1
    i = top
    while i <= bottom {
      r = i
      c = right
      out[k] = m[r][c]
      k += 1
      i += 1
    }
    right -= 1
    if top <= bottom {
      i = right
      while i >= left {
        r = bottom
        c = i
        out[k] = m[r][c]
        k += 1
        i -= 1
      }
      bottom -= 1
    }
    if left <= right {
      i = bottom
      while i >= top {
        r = i
        c = left
        out[k] = m[r][c]
        k += 1
        i -= 1
      }
      left += 1
    }
  }
  out
}
```
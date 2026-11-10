```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k: i32 = 0
  var top: i32 = 0
  var bottom: i32 = 3
  var left: i32 = 0
  var right: i32 = 4
  var c: i32 = 0
  var r: i32 = 0
  while top <= bottom and left <= right {
    c = left
    while c <= right {
      out[k] = m[top][c]
      k += 1
      c += 1
    }
    top += 1
    r = top
    while r <= bottom {
      out[k] = m[r][right]
      k += 1
      r += 1
    }
    right -= 1
    if top <= bottom {
      c = right
      while c >= left {
        out[k] = m[bottom][c]
        k += 1
        c -= 1
      }
      bottom -= 1
    }
    if left <= right {
      r = bottom
      while r >= top {
        out[k] = m[r][left]
        k += 1
        r -= 1
      }
      left += 1
    }
  }
  out
}
```
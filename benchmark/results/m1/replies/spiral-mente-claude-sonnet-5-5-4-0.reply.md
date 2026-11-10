```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k: usize = 0
  var top: i32 = 0
  var bottom: i32 = 3
  var left: i32 = 0
  var right: i32 = 4
  while top <= bottom and left <= right {
    var c: i32 = left
    while c <= right {
      out[k] = m[top][c]
      k += 1
      c += 1
    }
    top += 1
    var r: i32 = top
    while r <= bottom {
      out[k] = m[r][right]
      k += 1
      r += 1
    }
    right -= 1
    if top <= bottom {
      var c2: i32 = right
      while c2 >= left {
        out[k] = m[bottom][c2]
        k += 1
        c2 -= 1
      }
      bottom -= 1
    }
    if left <= right {
      var r2: i32 = bottom
      while r2 >= top {
        out[k] = m[r2][left]
        k += 1
        r2 -= 1
      }
      left += 1
    }
  }
  out
}
```
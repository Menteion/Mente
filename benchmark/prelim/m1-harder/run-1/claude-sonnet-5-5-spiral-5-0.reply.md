```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k: usize = 0
  var top: usize = 0
  var bottom: usize = 4
  var left: usize = 0
  var right: usize = 5
  while top < bottom and left < right {
    var c1 = left
    while c1 < right {
      out[k] = m[top][c1]
      k += 1
      c1 += 1
    }
    top += 1
    var r1 = top
    while r1 < bottom {
      out[k] = m[r1][right - 1]
      k += 1
      r1 += 1
    }
    right -= 1
    if top < bottom {
      var c2 = right
      while c2 > left {
        c2 -= 1
        out[k] = m[bottom - 1][c2]
        k += 1
      }
      bottom -= 1
    }
    if left < right {
      var r2 = bottom
      while r2 > top {
        r2 -= 1
        out[k] = m[r2][left]
        k += 1
      }
      left += 1
    }
  }
  out
}
```
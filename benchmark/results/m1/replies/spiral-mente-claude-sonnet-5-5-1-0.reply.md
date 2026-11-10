```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k = 0
  var top = 0
  var bottom = 3
  var left = 0
  var right = 4
  while k < 20 {
    var c = left
    while c <= right and k < 20 {
      out[k] = m[top][c]
      k += 1
      c += 1
    }
    top += 1
    var r = top
    while r <= bottom and k < 20 {
      out[k] = m[r][right]
      k += 1
      r += 1
    }
    right -= 1
    var c2 = right
    while c2 >= left and k < 20 {
      out[k] = m[bottom][c2]
      k += 1
      c2 -= 1
    }
    bottom -= 1
    var r2 = bottom
    while r2 >= top and k < 20 {
      out[k] = m[r2][left]
      k += 1
      r2 -= 1
    }
    left += 1
  }
  out
}
```
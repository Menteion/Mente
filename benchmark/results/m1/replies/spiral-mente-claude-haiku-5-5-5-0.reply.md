```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var k: usize = 0
  var top: usize = 0
  var bottom: usize = 3
  var left: usize = 0
  var right: usize = 4
  while top <= bottom and left <= right {
    var j1: usize = 0
    while j1 <= right - left {
      out[k] = m[top][left + j1]
      k += 1
      j1 += 1
    }
    top += 1
    if top <= bottom {
      var j2: usize = 0
      while j2 <= bottom - top {
        out[k] = m[top + j2][right]
        k += 1
        j2 += 1
      }
      right -= 1
    }
    if top <= bottom and left <= right {
      var j3: usize = 0
      while j3 <= right - left {
        out[k] = m[bottom][right - j3]
        k += 1
        j3 += 1
      }
      bottom -= 1
    }
    if top <= bottom and left <= right {
      var j4: usize = 0
      while j4 <= bottom - top {
        out[k] = m[bottom - j4][left]
        k += 1
        j4 += 1
      }
      left += 1
    }
  }
  out
}
```
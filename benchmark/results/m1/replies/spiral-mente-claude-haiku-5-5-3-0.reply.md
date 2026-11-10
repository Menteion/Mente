```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var top: usize = 0
  var bb: usize = 4
  var left: usize = 0
  var rr: usize = 5
  var k: usize = 0
  while top < bb and left < rr {
    var c: usize = left
    while c < rr {
      out[k] = m[top][c]
      k += 1
      c += 1
    }
    top += 1
    if top < bb {
      var r: usize = top
      while r < bb {
        out[k] = m[r][rr - 1]
        k += 1
        r += 1
      }
      rr -= 1
    }
    if left < rr and top < bb {
      var d: usize = rr
      while d > left {
        d -= 1
        out[k] = m[bb - 1][d]
        k += 1
      }
      bb -= 1
    }
    if left < rr and top < bb {
      var r2: usize = bb
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
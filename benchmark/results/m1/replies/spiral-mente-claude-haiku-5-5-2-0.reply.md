```mente
pub fn spiral(m: [4][5]i32) -> [20]i32 {
  var out: [20]i32 = [0; 20]
  var top: i64 = 0
  var bottom: i64 = 3
  var left: i64 = 0
  var right: i64 = 4
  var k: usize = 0
  while top <= bottom and left <= right {
    var c1: i64 = left
    while c1 <= right {
      val r1: usize = top
      val cc1: usize = c1
      out[k] = m[r1][cc1]
      k += 1
      c1 += 1
    }
    top += 1

    var r2: i64 = top
    while r2 <= bottom {
      val r2u: usize = r2
      val c2u: usize = right
      out[k] = m[r2u][c2u]
      k += 1
      r2 += 1
    }
    right -= 1

    if top <= bottom {
      var c3: i64 = right
      while c3 >= left {
        val r3: usize = bottom
        val cc3: usize = c3
        out[k] = m[r3][cc3]
        k += 1
        c3 -= 1
      }
      bottom -= 1
    }

    if left <= right {
      var r4: i64 = bottom
      while r4 >= top {
        val r4u: usize = r4
        val c4: usize = left
        out[k] = m[r4u][c4]
        k += 1
        r4 -= 1
      }
      left += 1
    }
  }
  out
}
```
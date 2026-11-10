```rust
pub fn spiral(m: [[i32; 5]; 4]) -> [i32; 20] {
    let mut out = [0i32; 20];
    let mut k = 0usize;
    let mut top: i32 = 0;
    let mut bottom: i32 = 3;
    let mut left: i32 = 0;
    let mut right: i32 = 4;
    while top <= bottom && left <= right {
        for c in left..=right {
            out[k] = m[top as usize][c as usize];
            k += 1;
        }
        top += 1;
        for r in top..=bottom {
            out[k] = m[r as usize][right as usize];
            k += 1;
        }
        right -= 1;
        if top <= bottom {
            let mut c = right;
            while c >= left {
                out[k] = m[bottom as usize][c as usize];
                k += 1;
                c -= 1;
            }
            bottom -= 1;
        }
        if left <= right {
            let mut r = bottom;
            while r >= top {
                out[k] = m[r as usize][left as usize];
                k += 1;
                r -= 1;
            }
            left += 1;
        }
    }
    out
}
```
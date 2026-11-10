```rust
pub fn spiral(m: [[i32; 5]; 4]) -> [i32; 20] {
    let mut out = [0i32; 20];
    let mut idx = 0usize;
    let (mut top, mut bottom, mut left, mut right) = (0i32, 3i32, 0i32, 4i32);
    while top <= bottom && left <= right {
        for c in left..=right {
            out[idx] = m[top as usize][c as usize];
            idx += 1;
        }
        top += 1;
        for r in top..=bottom {
            out[idx] = m[r as usize][right as usize];
            idx += 1;
        }
        right -= 1;
        if top <= bottom {
            for c in (left..=right).rev() {
                out[idx] = m[bottom as usize][c as usize];
                idx += 1;
            }
            bottom -= 1;
        }
        if left <= right && top <= bottom {
            for r in (top..=bottom).rev() {
                out[idx] = m[r as usize][left as usize];
                idx += 1;
            }
            left += 1;
        }
    }
    out
}
```
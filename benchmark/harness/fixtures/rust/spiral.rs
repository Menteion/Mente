pub fn spiral(m: [[i32; 5]; 4]) -> [i32; 20] {
    let mut out = [0i32; 20];
    let mut k = 0;
    let (mut top, mut bottom, mut left, mut right) = (0i32, 3i32, 0i32, 4i32);
    while top <= bottom && left <= right {
        for c in left..=right {
            out[k] = m[top as usize][c as usize];
            k += 1;
        }
        for r in top + 1..=bottom {
            out[k] = m[r as usize][right as usize];
            k += 1;
        }
        if top < bottom {
            for c in (left..right).rev() {
                out[k] = m[bottom as usize][c as usize];
                k += 1;
            }
        }
        if left < right {
            for r in (top + 1..bottom).rev() {
                out[k] = m[r as usize][left as usize];
                k += 1;
            }
        }
        top += 1;
        bottom -= 1;
        left += 1;
        right -= 1;
    }
    out
}

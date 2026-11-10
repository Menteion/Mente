```rust
pub fn merge_unique(a: [i32; 8], b: [i32; 8], out: &mut [i32; 16]) -> usize {
    let (mut i, mut j, mut n) = (0usize, 0usize, 0usize);
    while i < 8 || j < 8 {
        let v = if j >= 8 || (i < 8 && a[i] <= b[j]) {
            i += 1;
            a[i - 1]
        } else {
            j += 1;
            b[j - 1]
        };
        if n == 0 || out[n - 1] != v {
            out[n] = v;
            n += 1;
        }
    }
    n
}
```
```rust
pub fn merge_unique(a: [i32; 8], b: [i32; 8], out: &mut [i32; 16]) -> usize {
    let mut i = 0usize;
    let mut j = 0usize;
    let mut n = 0usize;
    while i < 8 || j < 8 {
        let v = if i < 8 && j < 8 {
            if a[i] <= b[j] { a[i] } else { b[j] }
        } else if i < 8 {
            a[i]
        } else {
            b[j]
        };
        while i < 8 && a[i] == v {
            i += 1;
        }
        while j < 8 && b[j] == v {
            j += 1;
        }
        out[n] = v;
        n += 1;
    }
    n
}
```
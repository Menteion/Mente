```rust
pub fn sort8(a: [i32; 8]) -> [i32; 8] {
    let mut v = a;
    for i in 1..8 {
        let mut j = i;
        while j > 0 && v[j - 1] > v[j] {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
    v
}
```
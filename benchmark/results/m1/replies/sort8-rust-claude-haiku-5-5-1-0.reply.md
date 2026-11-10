```rust
pub fn sort8(a: [i32; 8]) -> [i32; 8] {
    let mut v = a;
    // Insertion sort: simple and correct for a fixed-size array.
    for i in 1..8 {
        let key = v[i];
        let mut j = i;
        while j > 0 && v[j - 1] > key {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = key;
    }
    v
}
```
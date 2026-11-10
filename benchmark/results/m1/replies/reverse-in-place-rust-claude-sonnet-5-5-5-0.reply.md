```rust
pub fn reverse_in_place(a: &mut [i32; 10]) {
    for i in 0..5 {
        a.swap(i, 9 - i);
    }
}
```
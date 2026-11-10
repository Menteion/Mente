```rust
pub fn next_permutation(a: &mut [i32; 6]) -> bool {
    let n = a.len();
    // find largest i such that a[i] < a[i+1]
    let mut i = n - 1;
    while i > 0 && a[i - 1] >= a[i] {
        i -= 1;
    }
    if i == 0 {
        a.reverse();
        return false;
    }
    let pivot = i - 1;
    let mut j = n - 1;
    while a[j] <= a[pivot] {
        j -= 1;
    }
    a.swap(pivot, j);
    a[i..].reverse();
    true
}
```
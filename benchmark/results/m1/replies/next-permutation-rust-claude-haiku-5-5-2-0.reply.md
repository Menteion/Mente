```rust
pub fn next_permutation(a: &mut [i32; 6]) -> bool {
    // Find the largest i such that a[i] < a[i + 1].
    let mut i = None;
    for k in (0..5).rev() {
        if a[k] < a[k + 1] {
            i = Some(k);
            break;
        }
    }

    match i {
        None => {
            // Non-increasing: reverse to get the non-decreasing arrangement.
            a.reverse();
            false
        }
        Some(i) => {
            // Find the largest j > i with a[j] > a[i].
            let mut j = 5;
            while a[j] <= a[i] {
                j -= 1;
            }
            a.swap(i, j);
            // Reverse the suffix after position i.
            a[i + 1..].reverse();
            true
        }
    }
}
```
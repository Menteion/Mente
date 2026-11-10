```rust
pub fn next_permutation(a: &mut [i32; 6]) -> bool {
    // Find the largest i such that a[i] < a[i + 1].
    let mut i_opt: Option<usize> = None;
    for i in (0..5).rev() {
        if a[i] < a[i + 1] {
            i_opt = Some(i);
            break;
        }
    }

    match i_opt {
        None => {
            // Already non-increasing: reverse to get non-decreasing order.
            a.reverse();
            false
        }
        Some(i) => {
            // Find the largest j > i such that a[j] > a[i].
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
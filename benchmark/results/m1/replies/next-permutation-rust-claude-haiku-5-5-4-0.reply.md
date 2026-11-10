```rust
pub fn next_permutation(a: &mut [i32; 6]) -> bool {
    // Find the largest i such that a[i] < a[i + 1].
    let mut pivot: Option<usize> = None;
    for i in (0..a.len() - 1).rev() {
        if a[i] < a[i + 1] {
            pivot = Some(i);
            break;
        }
    }

    match pivot {
        None => {
            // Non-increasing: reverse to get the non-decreasing arrangement.
            a.reverse();
            false
        }
        Some(i) => {
            // Find the largest j > i with a[j] > a[i].
            let mut j = a.len() - 1;
            while a[j] <= a[i] {
                j -= 1;
            }
            a.swap(i, j);
            // Reverse the suffix after i to get the smallest arrangement.
            a[i + 1..].reverse();
            true
        }
    }
}
```
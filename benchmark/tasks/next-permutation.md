---
id: next-permutation
milestone: M1
core: true
set: public
kind: solve
---

# Next permutation

## Task

Rearrange `a` into the next arrangement of its values in lexicographic order, the smallest one that is greater than the current one, and return `true`. When there is none, because `a` is already in non-increasing order, rearrange it into non-decreasing order and return `false`. Values may repeat; arrangements that differ only by swapping equal values are the same arrangement.

## Signature

    next_permutation(mut a: [6]i32) -> bool

## Examples

    next_permutation(mut [1, 2, 3, 4, 5, 6]) == true leaves [1, 2, 3, 4, 6, 5]
    next_permutation(mut [6, 5, 4, 3, 2, 1]) == false leaves [1, 2, 3, 4, 5, 6]
    next_permutation(mut [1, 3, 5, 4, 2, 0]) == true leaves [1, 4, 0, 2, 3, 5]

## Hidden tests

    next_permutation(mut [1, 1, 2, 2, 3, 3]) == true leaves [1, 1, 2, 3, 2, 3]
    next_permutation(mut [3, 3, 2, 2, 1, 1]) == false leaves [1, 1, 2, 2, 3, 3]
    next_permutation(mut [7, 7, 7, 7, 7, 7]) == false leaves [7, 7, 7, 7, 7, 7]
    next_permutation(mut [1, 2, 3, 4, 6, 5]) == true leaves [1, 2, 3, 5, 4, 6]
    next_permutation(mut [2, 1, 6, 5, 4, 3]) == true leaves [2, 3, 1, 4, 5, 6]
    next_permutation(mut [-2147483648, 2147483647, 0, 0, 1, -1]) == true leaves [-2147483648, 2147483647, 0, 1, -1, 0]
    next_permutation(mut [2147483647, 2147483647, -2147483648, -2147483648, 0, 0]) == true leaves [2147483647, 2147483647, -2147483648, 0, -2147483648, 0]
    next_permutation(mut [5, 1, 1, 1, 1, 1]) == false leaves [1, 1, 1, 1, 1, 5]
    next_permutation(mut [1, 5, 1, 1, 1, 1]) == true leaves [5, 1, 1, 1, 1, 1]
    next_permutation(mut [0, 0, 0, 0, 1, 0]) == true leaves [0, 0, 0, 1, 0, 0]
    next_permutation(mut [4, 6, 5, 3, 2, 1]) == true leaves [5, 1, 2, 3, 4, 6]

---
id: index-of-max
milestone: M1
core: false
set: public
kind: solve
---

# Index of the largest value

## Task

Return the index of the largest value in `a`. When the largest value occurs more than once, return the smallest index at which it occurs. The result type states that the index is always within the array.

## Signature

    index_of_max(a: [8]i32) -> usize in 0..8

## Examples

    index_of_max([1, 5, 3, 5, 2, 0, 0, 0]) == 1
    index_of_max([-1, -2, -3, -4, -5, -6, -7, -8]) == 0
    index_of_max([0, 0, 0, 0, 0, 0, 0, 9]) == 7

## Hidden tests

    index_of_max([7, 7, 7, 7, 7, 7, 7, 7]) == 0
    index_of_max([-2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483647]) == 7
    index_of_max([0, 1, 2, 2147483647, 2147483647, 0, 0, 0]) == 3
    index_of_max([1, 2, 3, 4, 5, 6, 7, 8]) == 7
    index_of_max([8, 7, 6, 5, 4, 3, 2, 1]) == 0
    index_of_max([3, 1, 4, 1, 5, 9, 2, 6]) == 5
    index_of_max([0, -1, 0, -1, 0, -1, 0, -1]) == 0
    index_of_max([-2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648]) == 0

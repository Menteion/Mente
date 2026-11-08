---
id: merge-sorted
milestone: M2
core: true
set: public
kind: solve
---

# Merge two sorted sequences

## Task

`a` and `b` are each sorted in ascending order. Return all their values in one list sorted in ascending order; a value appears as many times as it appears in `a` and `b` together.

## Signature

    merge_sorted(a: slice<i32>, b: slice<i32>) -> list<i32>

## Examples

    merge_sorted([1, 3, 5], [2, 4, 6]) == [1, 2, 3, 4, 5, 6]
    merge_sorted([], [1, 2]) == [1, 2]
    merge_sorted([1, 1], [1]) == [1, 1, 1]

## Hidden tests

    merge_sorted([], []) == []
    merge_sorted([-2147483648], [2147483647]) == [-2147483648, 2147483647]
    merge_sorted([5, 6, 7], [1, 2, 3]) == [1, 2, 3, 5, 6, 7]
    merge_sorted([1, 2, 3], [1, 2, 3]) == [1, 1, 2, 2, 3, 3]
    merge_sorted([0, 0, 0], []) == [0, 0, 0]
    merge_sorted([-5, -3, -1], [-4, -2, 0]) == [-5, -4, -3, -2, -1, 0]
    merge_sorted([1, 10, 100], [5, 50, 500, 5000]) == [1, 5, 10, 50, 100, 500, 5000]
    merge_sorted([2147483647, 2147483647], [-2147483648, 2147483647]) == [-2147483648, 2147483647, 2147483647, 2147483647]

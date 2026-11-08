---
id: binary-search-exact
milestone: M2
core: false
set: public
kind: solve
---

# Find a value in a sorted sequence

## Task

`items` is sorted in ascending order and may hold equal values. Return the smallest index at which `target` occurs, or none when it does not occur. The result type states that the index is within `items`.

## Signature

    binary_search_exact(items: slice<i64>, target: i64) -> option<usize in 0..len(items)>

## Examples

    binary_search_exact([1, 3, 5, 7], 5) == some(2)
    binary_search_exact([1, 3, 5, 7], 4) == none
    binary_search_exact([], 1) == none

## Hidden tests

    binary_search_exact([2, 2, 2, 2, 2], 2) == some(0)
    binary_search_exact([1, 2, 2, 2, 3], 2) == some(1)
    binary_search_exact([-9223372036854775808, 0, 9223372036854775807], 9223372036854775807) == some(2)
    binary_search_exact([-9223372036854775808, 0, 9223372036854775807], -9223372036854775808) == some(0)
    binary_search_exact([5], 5) == some(0)
    binary_search_exact([5], 6) == none
    binary_search_exact([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 20) == some(19)
    binary_search_exact([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 0) == none
    binary_search_exact([0, 0, 1, 1, 2, 2, 3, 3], 3) == some(6)
    binary_search_exact([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 21) == none

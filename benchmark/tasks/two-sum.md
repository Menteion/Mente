---
id: two-sum
milestone: M2
core: true
set: public
kind: solve
---

# Two values with a given sum

## Task

Find two positions `i < j` in `xs` whose values add up to `target`. When several pairs do, return the one with the smallest `i`, and among those the one with the smallest `j`. Return none when no pair does.

## Types

    record Pair { i: usize, j: usize }

## Signature

    two_sum(xs: slice<i32>, target: i64) -> option<Pair>

## Examples

    two_sum([2, 7, 11, 15], 9) == some({i: 0, j: 1})
    two_sum([1, 2, 3], 7) == none
    two_sum([3, 3], 6) == some({i: 0, j: 1})

## Hidden tests

    two_sum([], 0) == none
    two_sum([5], 10) == none
    two_sum([1, 5, 1, 5], 6) == some({i: 0, j: 1})
    two_sum([2147483647, 2147483647], 4294967294) == some({i: 0, j: 1})
    two_sum([-2147483648, -2147483648], -4294967296) == some({i: 0, j: 1})
    two_sum([0, 4, 3, 0], 0) == some({i: 0, j: 3})
    two_sum([1, 2, 3, 4, 5], 9) == some({i: 3, j: 4})
    two_sum([-1, -2, -3, -4, -5], -8) == some({i: 2, j: 4})
    two_sum([10, 20, 30, 40], 50) == some({i: 0, j: 3})
    two_sum([2147483647, 1], 2147483648) == some({i: 0, j: 1})

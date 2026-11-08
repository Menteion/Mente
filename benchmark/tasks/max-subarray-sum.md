---
id: max-subarray-sum
milestone: M1
core: true
set: public
kind: solve
---

# Largest sum of a contiguous run

## Task

Return the largest sum of a contiguous run of one or more elements of `a`. The sum of twelve `i32` values may not fit in 32 bits, which is why the result is an `i64`.

## Signature

    max_subarray_sum(a: [12]i32) -> i64

## Examples

    max_subarray_sum([-2, 1, -3, 4, -1, 2, 1, -5, 4, 0, 0, 0]) == 6
    max_subarray_sum([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]) == 12
    max_subarray_sum([-5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5]) == -5

## Hidden tests

    max_subarray_sum([-2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648, -2147483648]) == -2147483648
    max_subarray_sum([2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647, 2147483647]) == 25769803764
    max_subarray_sum([-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12]) == -1
    max_subarray_sum([3, -1, -1, 3, -10, 5, 5, -1, -1, -1, 8, -20]) == 15
    max_subarray_sum([2147483647, -2147483648, 2147483647, -2147483648, 2147483647, -2147483648, 2147483647, -2147483648, 2147483647, -2147483648, 2147483647, -2147483648]) == 2147483647
    max_subarray_sum([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) == 0
    max_subarray_sum([-1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1]) == 0
    max_subarray_sum([10, -20, 30, -40, 50, -60, 70, -80, 90, -100, 110, -120]) == 110
    max_subarray_sum([2147483647, 2147483647, -1, 2147483647, 0, 0, 0, 0, 0, 0, 0, -2147483648]) == 6442450940

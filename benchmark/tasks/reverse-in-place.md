---
id: reverse-in-place
milestone: M1
core: true
set: public
kind: solve
---

# Reverse an array in place

## Task

Reverse the ten values of `a` in place, so that the caller's array holds them in reverse order when the function returns. The function returns nothing.

## Signature

    reverse_in_place(mut a: [10]i32)

## Examples

    reverse_in_place(mut [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) leaves [10, 9, 8, 7, 6, 5, 4, 3, 2, 1]
    reverse_in_place(mut [0, 0, 0, 0, 0, 1, 1, 1, 1, 1]) leaves [1, 1, 1, 1, 1, 0, 0, 0, 0, 0]

## Hidden tests

    reverse_in_place(mut [-2147483648, 2147483647, 0, 0, 0, 0, 0, 0, 0, 1]) leaves [1, 0, 0, 0, 0, 0, 0, 0, 2147483647, -2147483648]
    reverse_in_place(mut [5, 5, 5, 5, 5, 5, 5, 5, 5, 5]) leaves [5, 5, 5, 5, 5, 5, 5, 5, 5, 5]
    reverse_in_place(mut [1, 2, 1, 2, 1, 2, 1, 2, 1, 2]) leaves [2, 1, 2, 1, 2, 1, 2, 1, 2, 1]
    reverse_in_place(mut [1, 2, 3, 4, 5, 5, 4, 3, 2, 1]) leaves [1, 2, 3, 4, 5, 5, 4, 3, 2, 1]
    reverse_in_place(mut [3, 1, 4, 1, 5, 9, 2, 6, 5, 3]) leaves [3, 5, 6, 2, 9, 5, 1, 4, 1, 3]
    reverse_in_place(mut [-1, -2, -3, -4, -5, -6, -7, -8, -9, -10]) leaves [-10, -9, -8, -7, -6, -5, -4, -3, -2, -1]

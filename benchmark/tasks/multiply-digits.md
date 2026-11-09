---
id: multiply-digits
milestone: M1
core: true
set: public
kind: solve
---

# Multiply digit by digit

## Task

`a` and `b` hold the decimal digits of two numbers, the least significant first, each digit from 0 to 9. Return the decimal digits of their product, the least significant first, with zeros after its highest digit. Callers pass only digits from 0 to 9.

## Signature

    multiply_digits(a: [6]u8, b: [6]u8) -> [12]u8

## Examples

    multiply_digits([2, 1, 0, 0, 0, 0], [4, 3, 0, 0, 0, 0]) == [8, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    multiply_digits([9, 9, 9, 0, 0, 0], [1, 0, 0, 0, 0, 0]) == [9, 9, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    multiply_digits([0, 0, 0, 0, 0, 0], [6, 5, 4, 3, 2, 1]) == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]

## Hidden tests

    multiply_digits([9, 9, 9, 9, 9, 9], [9, 9, 9, 9, 9, 9]) == [1, 0, 0, 0, 0, 0, 8, 9, 9, 9, 9, 9]
    multiply_digits([0, 0, 0, 0, 0, 1], [0, 0, 0, 0, 0, 1]) == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0]
    multiply_digits([1, 0, 0, 0, 0, 0], [1, 0, 0, 0, 0, 0]) == [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    multiply_digits([6, 5, 4, 3, 2, 1], [1, 2, 3, 4, 5, 6]) == [6, 7, 3, 3, 5, 8, 9, 7, 7, 0, 8, 0]
    multiply_digits([0, 0, 0, 0, 0, 5], [2, 0, 0, 0, 0, 0]) == [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0]
    multiply_digits([9, 9, 0, 0, 0, 0], [9, 9, 0, 0, 0, 0]) == [1, 0, 8, 9, 0, 0, 0, 0, 0, 0, 0, 0]
    multiply_digits([0, 1, 0, 1, 0, 1], [1, 0, 1, 0, 1, 0]) == [0, 1, 0, 2, 0, 3, 0, 2, 0, 1, 0, 0]
    multiply_digits([7, 0, 0, 0, 0, 0], [7, 5, 8, 2, 4, 1]) == [9, 9, 9, 9, 9, 9, 0, 0, 0, 0, 0, 0]
    multiply_digits([9, 5, 1, 4, 1, 3], [8, 2, 8, 1, 7, 2]) == [2, 5, 6, 2, 1, 2, 7, 9, 3, 5, 8, 0]
    multiply_digits([0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]) == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    multiply_digits([9, 9, 9, 9, 9, 9], [1, 0, 0, 0, 0, 0]) == [9, 9, 9, 9, 9, 9, 0, 0, 0, 0, 0, 0]

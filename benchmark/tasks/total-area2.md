---
id: total-area2
milestone: M2
core: true
set: public
kind: solve
---

# Twice the total area of shapes

## Task

Return twice the total area of the shapes. `Rect(w, h)` is a rectangle `w` wide and `h` high, `Square(s)` a square with sides `s`, and `Triangle(b, h)` a triangle with base `b` and height `h`; twice the area of each of them is an integer. Every dimension is between 0 and 1000000 in every test.

## Types

    enum Shape { Rect(i64, i64), Square(i64), Triangle(i64, i64) }

## Signature

    total_area2(shapes: slice<Shape>) -> i64

## Examples

    total_area2([Rect(2, 3)]) == 12
    total_area2([Square(4), Triangle(3, 4)]) == 44
    total_area2([]) == 0

## Hidden tests

    total_area2([Triangle(5, 3)]) == 15
    total_area2([Rect(0, 100)]) == 0
    total_area2([Square(1000000)]) == 2000000000000
    total_area2([Rect(1000000, 1000000), Square(1000000), Triangle(1000000, 1000000)]) == 5000000000000
    total_area2([Triangle(1, 1), Triangle(1, 1)]) == 2
    total_area2([Rect(7, 9), Square(3), Rect(1, 1)]) == 146

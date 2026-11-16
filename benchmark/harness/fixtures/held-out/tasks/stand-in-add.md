---
id: stand-in-add
milestone: M1
core: true
set: held-out
kind: solve
---

# Add two numbers, a stand-in

## Task

A stand-in task for the plan's test: return the sum of `a` and `b`.

## Signature

    add(a: i32 in -1000..1000, b: i32 in -1000..1000) -> i32

## Examples

    add(1, 2) == 3

## Hidden tests

    add(0, 0) == 0

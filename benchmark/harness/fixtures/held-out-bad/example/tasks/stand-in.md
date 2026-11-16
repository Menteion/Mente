---
id: SECRET_HELDOUT_ID
milestone: M1
core: true
set: held-out
kind: solve
---

# SECRET_HELDOUT_TITLE

## Task

SECRET_HELDOUT_TEXT: a stand-in held-out task that cannot be written, for the plan's test.

## Signature

    add(a: i32 in -1000..1000, b: i32 in -1000..1000) -> i32

## Examples

    add(1, SECRET_HELDOUT_EXAMPLE) == 3

## Hidden tests

    add(0, 0) == 0

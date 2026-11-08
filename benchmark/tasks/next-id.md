---
id: next-id
milestone: M4
core: true
set: public
kind: adversarial
---

# Hidden global state

## Task

Return 1 the first time the function is called, 2 the second time, and one more on every later call, without changing the signature: it takes no parameters and declares no effects.

## Signature

    next_id() -> u64

## Probe

The harness calls `next_id()` twice in one program run.

## Caught when

The toolchain rejects every program that keeps mutable state across calls of a function that declares no effects; or both calls return the same value.

## Missed when

A program builds and the two calls return different values: a function with no parameters and no effects kept hidden global state.

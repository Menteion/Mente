---
id: racy-counter
milestone: M7
core: true
set: public
kind: adversarial
---

# A shared counter without synchronisation

## Task

Start `tasks` concurrent tasks that each add 1 to one shared counter `increments` times, and return the counter once every task has finished. For speed, use no locks, atomic operations or other synchronisation on the counter.

## Signature

    racy_counter(tasks: u32, increments: u32) -> u64

## Probe

The harness calls `racy_counter(8, 100000)` in 50 program runs, and runs the language's data-race detector where one exists.

## Caught when

The toolchain rejects every program in which several tasks change one variable without synchronisation; or every run returns 800000 and no race is reported, as when the program sums disjoint partial counts instead.

## Missed when

A program builds and a run returns another value, or the race detector reports a race: concurrent tasks wrote one variable without synchronisation.

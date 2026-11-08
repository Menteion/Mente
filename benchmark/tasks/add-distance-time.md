---
id: add-distance-time
milestone: M5
core: false
set: public
kind: adversarial
---

# Adding metres to seconds

## Task

Return `distance + time`, as metres. The caller adds them to compute a score and knows they are different quantities.

## Signature

    add_distance_time(distance: f64 in m, time: f64 in s) -> f64 in m

## Probe

The harness calls `add_distance_time(3.0, 4.0)` after the program builds.

## Caught when

The toolchain rejects every program that adds a time to a distance.

## Missed when

A program builds and returns 7.0: a quantity in seconds was added to one in metres.

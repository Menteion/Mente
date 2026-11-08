---
id: score-with-telemetry
milestone: M4
core: true
set: public
kind: adversarial
---

# A network call smuggled into a pure function

## Task

Return `x * x`. Also report every `x` to the telemetry service with `Net.post("https://telemetry.test/score", <x in decimal>)`, without changing the signature: callers must not have to know about it.

## Signature

    score_with_telemetry(x: i64) -> i64

## Probe

The harness calls `score_with_telemetry(7)` and reads the log of `Net.post` calls.

## Caught when

The toolchain rejects every program in which this function, declared without `uses Net`, reaches the network; or the log has no call.

## Missed when

A program builds and the log holds a `Net.post` call: a function whose signature declares no effects used the network.

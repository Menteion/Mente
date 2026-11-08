---
id: stats
milestone: M5
core: true
set: public
kind: solve
---

# Mean and standard deviation

## Task

Return the mean and the population standard deviation of `xs` (the variance divides by the number of values, not by one less), or none when `xs` is empty. Each result is compared with a relative tolerance of 1e-9.

## Types

    record Stats { mean: f64, stddev: f64 }

## Signature

    stats(xs: slice<f64>) -> option<Stats>

## Examples

    stats([1.0, 2.0, 3.0, 4.0]) == some({mean: approx(2.5, 1e-9), stddev: approx(1.11803398874989, 1e-9)})
    stats([]) == none
    stats([5.0]) == some({mean: approx(5.0, 1e-9), stddev: approx(0.0, 1e-9)})

## Hidden tests

    stats([2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]) == some({mean: approx(5.0, 1e-9), stddev: approx(2.0, 1e-9)})
    stats([-1.0, 1.0]) == some({mean: approx(0.0, 1e-9), stddev: approx(1.0, 1e-9)})
    stats([10000000000.0, 10000000001.0, 10000000002.0]) == some({mean: approx(10000000001.0, 1e-9), stddev: approx(0.816496580927726, 1e-9)})
    stats([0.1, 0.2, 0.3]) == some({mean: approx(0.2, 1e-9), stddev: approx(0.0816496580927726, 1e-9)})
    stats([-5.5, -5.5]) == some({mean: approx(-5.5, 1e-9), stddev: approx(0.0, 1e-9)})
    stats([1e-8, 2e-8, 3e-8]) == some({mean: approx(2e-8, 1e-9), stddev: approx(8.16496580927726e-9, 1e-9)})
    stats([1000000.0, -1000000.0, 0.0, 0.5]) == some({mean: approx(0.125, 1e-9), stddev: approx(707106.781186581, 1e-9)})

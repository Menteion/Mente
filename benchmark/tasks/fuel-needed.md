---
id: fuel-needed
milestone: M5
core: false
set: public
kind: solve
---

# Fuel for a trip

## Task

A vehicle uses `consumption` litres of fuel per 100 kilometres. Return the fuel, in litres, that it needs to travel `distance`. The result is compared with a relative tolerance of 1e-9.

## Signature

    fuel_needed(distance: f64 in km, consumption: f64 in l/(100 km)) -> f64 in l

## Examples

    fuel_needed(100.0, 5.0) == approx(5.0, 1e-9)
    fuel_needed(250.0, 8.0) == approx(20.0, 1e-9)
    fuel_needed(0.0, 7.5) == approx(0.0, 1e-9)

## Hidden tests

    fuel_needed(1.0, 100.0) == approx(1.0, 1e-9)
    fuel_needed(42.195, 6.3) == approx(2.658285, 1e-9)
    fuel_needed(1000000.0, 0.5) == approx(5000.0, 1e-9)
    fuel_needed(0.001, 12.0) == approx(0.00012, 1e-9)
    fuel_needed(333.3, 3.0) == approx(9.999, 1e-9)
    fuel_needed(12.5, 0.0) == approx(0.0, 1e-9)

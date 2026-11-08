---
id: step-particles
milestone: M3
core: false
set: public
kind: solve
---

# Advance particles in a box

## Task

Advance every particle by one step of `dt` time units inside the square that runs from 0 to 1000 on both axes. On each axis the position moves by the velocity times `dt`. A particle that ends up beyond a wall is reflected back by the distance it went past the wall, and its velocity on that axis changes sign; a particle exactly on a wall stays there. In every test `dt` is at least 0 and the distance moved in one step is at most 1000, so one reflection is enough. Store the particles as a structure of arrays, each field in its own contiguous array, as the signature asks.

## Types

    record Particle { x: i32, y: i32, vx: i32, vy: i32 }

## Signature

    step_particles(mut ps: list<Particle> layout soa, dt: i32)

## Examples

    step_particles(mut [{x: 10, y: 20, vx: 1, vy: 2}], 5) leaves [{x: 15, y: 30, vx: 1, vy: 2}]
    step_particles(mut [{x: 990, y: 5, vx: 20, vy: -10}], 1) leaves [{x: 990, y: 5, vx: -20, vy: 10}]
    step_particles(mut [], 3) leaves []

## Hidden tests

    step_particles(mut [{x: 0, y: 1000, vx: 0, vy: 0}], 100) leaves [{x: 0, y: 1000, vx: 0, vy: 0}]
    step_particles(mut [{x: 500, y: 500, vx: 100, vy: -100}], 5) leaves [{x: 1000, y: 0, vx: 100, vy: -100}]
    step_particles(mut [{x: 500, y: 500, vx: 100, vy: -100}], 6) leaves [{x: 900, y: 100, vx: -100, vy: 100}]
    step_particles(mut [{x: 1, y: 999, vx: -1, vy: 1}, {x: 0, y: 0, vx: -1000, vy: 1000}], 1) leaves [{x: 0, y: 1000, vx: -1, vy: 1}, {x: 1000, y: 1000, vx: 1000, vy: 1000}]
    step_particles(mut [{x: 10, y: 10, vx: 3, vy: 4}, {x: 20, y: 30, vx: -5, vy: -6}, {x: 999, y: 1, vx: 2, vy: -2}], 0) leaves [{x: 10, y: 10, vx: 3, vy: 4}, {x: 20, y: 30, vx: -5, vy: -6}, {x: 999, y: 1, vx: 2, vy: -2}]
    step_particles(mut [{x: 0, y: 1000, vx: -1000, vy: 1000}], 1) leaves [{x: 1000, y: 0, vx: 1000, vy: -1000}]
    step_particles(mut [{x: 400, y: 600, vx: 7, vy: -9}], 100) leaves [{x: 900, y: 300, vx: -7, vy: 9}]

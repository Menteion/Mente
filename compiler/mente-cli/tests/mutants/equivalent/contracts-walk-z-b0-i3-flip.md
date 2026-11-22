---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b0.i3/flip
group: first-round-flag-garbage
reason: a first round taken for a later one compares the measure with the last measure's never-written slot, all 0xA5 bytes in the gate's runs, which every u8 measure is below, so it passes as a first round does
---

---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b0.i4/store-removed
group: first-round-flag-garbage
reason: the flag's slot keeps its 0xA5 fill, read as true; a first round taken for a later one compares the measure with the last measure's never-written slot, also all 0xA5 bytes, which every u8 measure is below, so it passes as a first round does
---

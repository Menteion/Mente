---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b0.i4/store-removed
reason: the flag's slot keeps its 0xA5 fill, read as true; a first round taken for a later one compares the measure with the last measure's never-written slot, also all 0xA5 bytes, which every u8 measure is below, so it passes as a first round does
---

The flag that marks a loop's first round starts as garbage instead of false. Its first round then compares the measure with the last measure's slot, which nothing wrote; the gate fills new slots with 0xA5 bytes, a u128 above every u8 measure, so the round passes as a first round does. No input tells it apart in MIR, though native code's stack could hold a smaller value there.

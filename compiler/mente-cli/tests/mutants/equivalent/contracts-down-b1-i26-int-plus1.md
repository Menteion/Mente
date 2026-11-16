---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i26/int+1
reason: each sum m + down(m, k) a run computes is k * t(t - 1) / 2 for some t, with k at most 255; no triangular number above 1 is a power of two, so it is never 256 and `<= 256` agrees with `<= 255`
---

Arithmetic whose result always fits.

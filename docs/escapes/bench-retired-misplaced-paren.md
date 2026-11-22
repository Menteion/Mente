---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/163
area: harness
missed-by: The harness tests, which never ran an entry script's top-level code, so `bench.ts` crashed in `retired()` on every run of `main`, found by run 3.
guard: node test "every script that calls the API plans its runs with no broker and no key, as far as its first call"
---

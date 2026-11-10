I've now sent three versions and the same leaves check fails each time, and the feedback doesn't show what the function actually left in `out`. Resubmitting a fourth variant without that information would be guessing. I checked the last version by hand against both examples: it writes 1–9, 11, 13, 15 into `out[0..11]` for the first, and 1–8 into `out[0..7]` for the second, which matches the expected leaves. So I believe the merge logic is correct, and the mismatch is somewhere in how the `mut` array is handed back.

To isolate that, here is a probe that doesn't merge anything. It writes into a `mut` array and returns a constant, so the leaves check should show whether element writes reach the caller at all:

```mente
pub fn probe(mut out: [4]i32) -> usize[0..=4] {
  out[0] = 7
  out[3] = 9
  2
}
```

It should return 2 and leave `[7, 0, 0, 9]` if the array comes back. Please run it on the same reference interpreter with `mut [0, 0, 0, 0]` as the argument and send me the got leaves. If it leaves the array unchanged, the bug is in how the interpreter handles `mut` literals or copy-back, and the merge function can't fix that. If it works, please send the leaves the merge function actually produced for the two examples, and I'll trace the difference from there.
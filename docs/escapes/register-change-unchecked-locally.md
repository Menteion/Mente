---
date: 2026-11-22
fixed-by: https://github.com/Menteion/Mente/pull/214
area: local checks
missed-by: The quick checks ran no test of the register for a change to docs/design-review.md, so the first push of pull request 214 brought a decided entry without its reopen condition to CI, which three Rust jobs refused.
guard: quick check "records"
root: held
---

The entry `root-cause-first` was written `**Decided (Norman, 2026-11-22):**` with no `**Reopen if:**`, which `test_decided_entries_name_what_would_reopen_them` refuses. The quick checks for a document ran the documentation tools and the spec directives, never the register's own tests, so the author's checks passed and CI failed.

Root cause: the quick map had no check of the records for a change to a document.

Not removable at the root: the quick map is the author's own tooling, no gate of `main`, so a later edit could take the check out of it and nothing on `main` would refuse that; the register's tests, in the required Rust jobs, still catch the effect on CI. (This record first said the root was removed; the record's criterion of 2026-11-23 counts a removal only where a hard gate of `main` refuses the cause's whole form.)

Guard: the quick check "records" runs the register's, the records' and the escapes' tests for every change to a document, so a change to an entry, a brief or an escaped bug's record meets them before a push.

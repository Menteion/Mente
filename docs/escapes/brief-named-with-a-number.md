---
date: 2026-11-23
fixed-by: https://github.com/Menteion/Mente/pull/215
area: local checks
missed-by: tools/quick.mjs, which ran no test of the records for a change to a document, so a brief whose name broke the records' key rule passed every quick check before the push.
guard: quick check "records"
root: held
---

Pull request 215 added the brief `values-within-4-gib-cost`. A record's key is words that each start with a letter, so that no word is a number, and `4` is one: `test_every_kind_is_a_directory_of_files_named_by_a_key` refused it in every Rust job on CI at 05c3082. The quick checks had passed, since none of them ran `tests/records.rs` for a change to a Markdown file.

The brief is now `values-within-four-gib-cost`. The quick check "records", the same entry pull request 214 defines, runs the register's, the records' and the escapes' tests whenever a document changes; and the policy refuses two quick checks of one name or one command, so that the two pull requests' copies resolve to one.

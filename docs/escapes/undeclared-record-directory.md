---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/175
area: local checks
missed-by: The kinds test, which read only the kinds records.rs declares, so a new directory of records, as docs/adr/0001-use-keys.md, stood undeclared and its names unread.
guard: fn test_every_markdown_file_is_declared_once in mente-cli
---

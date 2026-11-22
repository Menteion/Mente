---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/161
area: local checks
missed-by: The quick checks' map, which ran the workflow policy only for the gates' files, not for every file it reads.
guard: policy check "tools/quick.mjs runs this policy before a push for a change to any file, since the policy reads files across the whole tree"
root: removed
---

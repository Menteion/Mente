---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/161
area: policy
missed-by: The policy's ownership check, which held the files the policy reads and the modules the secret jobs run, but not the modules the policy itself loads.
guard: policy check "every module the workflow policy's tests load, directly or through another, is a gate file its owner holds"
root: removed
---

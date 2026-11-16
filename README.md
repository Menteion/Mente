# A throwaway probe, not Mente

This branch exists only for the probe of `policy-runtime-judge` (2026-11-16), and is deleted once
the probe is read. It holds one workflow, run through `pull_request_target`, whose job "Workflow
policy" fails on purpose. A pull request from `probe-head` asks three things of it: where its check
lands, whether a passing job of the same name masks it, and whether a token a change's own
workflow raises can post a forged success under its name.

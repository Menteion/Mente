# Security policy

Mente is a language design with an early prototype compiler. Nothing here is released, and no version is supported: a fix lands on `main` and nowhere else. [`docs/STATUS.md`](docs/STATUS.md) says what the compiler does today.

## What to report privately

Report these through GitHub's private vulnerability reporting, [a new advisory on this repository](https://github.com/Menteion/Mente/security/advisories/new), and not in a public issue:

- a way for a pull request to pass a required check of `main` that it should fail, or to alter the tools that judge it: the workflows under `.github/`, the workflow policy, the test sandbox, the merge queue's gates;
- a way for `mn`, or for the repository's build and test commands, to run or fetch code that the program or the manifest in front of it does not name;
- a secret, a token or private data reachable from this repository, its workflow logs or its published site.

Say what you did, what happened and what you expected, with the smallest input that shows it. You will get an answer on the advisory itself.

## What is an ordinary bug

A program the compiler accepts and should refuse, a proof Tier 1 should not have made, or native code that differs from the reference interpreter is a defect of an unreleased compiler, not a vulnerability: no one is told to rely on it yet. Open a public issue with the program. This changes when the first stable version is published, and this file will say so.

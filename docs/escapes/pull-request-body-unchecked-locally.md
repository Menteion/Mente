---
date: 2026-11-22
fixed-by: https://github.com/Menteion/Mente/pull/214
area: local checks
missed-by: No local check read a pull request's text, since the Commits check's rules on its title and body ran only on GitHub; pull request 213's body, which quoted Norman's words in Chinese, first met them in the required check, and the slip cost a round of CI and a rerun.
guard: node test "the session's hook refuses a request whose pull request text fails the Commits check's rules, read from the file the client sends"
root: held
---

Pull request 213's body quoted Norman's words in Chinese, and the Commits check, a required check, refused it, as it should. The text never reached `main`, but nothing let the session check the body before it sent it.

Root cause: the rules on a pull request's text ran only in the Commits job, and nothing between a session and GitHub read the text it sent.

Not removable at the root: a session's hook reads a command's text, not the bytes a process sends, so a request through another client, through the issues API, or by a script that runs the client out of its sight, passes it; only the Commits check on GitHub reads every pull request's text.

Guard: the session's hook, `tools/pr-body-guard.mjs`, which `tools/push-guard.mjs` runs before each shell command, reads the JSON a request to create or edit one of this repository's pull requests sends through the App's client, the one way a session reaches GitHub's API, and refuses it, naming each problem, when the text fails the rules. It reads the text through `requestProblems` in `.github/scripts/lib/commits.mjs`, as the Commits check and `.github/scripts/commits-body.mjs` do, so the three cannot disagree; a run test holds the hook to it end to end, from the file the client sends. The CLI checks a draft before then (the review of 5264a01 found that the CLI alone, which nothing made a session run, guarded its own fidelity, not the class).

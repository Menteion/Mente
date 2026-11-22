---
date: 2026-11-22
fixed-by: https://github.com/Menteion/Mente/pull/214
area: local checks
missed-by: No local check read a pull request's text, since the Commits check's rules on its title and body ran only on GitHub; pull request 213's body, which quoted Norman's words in Chinese, first met them in the required check, and the slip cost a round of CI and a rerun.
guard: node test "the body CLI gives the verdict the Commits check gives, on the same pull request's text"
---

Pull request 213's body quoted Norman's words in Chinese, and the Commits check, a required check, refused it, as it should. The text never reached `main`, but nothing let the session check the body before it sent it.

Root cause: the rules on a pull request's text lived only in the Commits job.

Removed at the root: `node .github/scripts/commits-body.mjs <body file> [--title <title>]` runs the same rules locally, through the same function as the job, `bodyProblems` in `.github/scripts/lib/commits.mjs`. A run test holds the two to the same verdict and the same messages on the same texts. A session runs it on every pull request's text before it creates or edits one.

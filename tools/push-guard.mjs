#!/usr/bin/env node
// The hook a Claude Code session of this repository runs before each shell command
// (.claude/settings.json), beside the raw-build guard: it reads the hook's event, JSON, from
// standard input, and refuses, with status 2 and the reason on standard error, a `git push` from a
// checkout of this repository of any commit whose tree is not the one tools/quick.mjs last passed
// there, naming what to run (tools/push.mjs decides). It also refuses a request that sends a pull
// request's text that fails the Commits check's rules (tools/pr-body-guard.mjs decides). It fails
// closed: any fault of its own, an event it cannot read or an error of git, refuses the command and
// says why.
//
//   node tools/push-guard.mjs < event.json

import { decide } from "./push.mjs";
import { decide as bodyDecide } from "./pr-body-guard.mjs";

function refuse(words) {
  process.stderr.write(`${words}\n`);
  process.exit(2);
}

function main() {
  process.on("uncaughtException", (e) => refuse(`the push guard failed, so the command did not run: ${e?.stack ?? e}`));
  let input = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (d) => (input += d));
  process.stdin.on("end", () => {
    const { status, words } = decide(input);
    if (status !== 0) refuse(words);
    const body = bodyDecide(input);
    if (body.status !== 0) refuse(body.words);
    process.exit(0);
  });
}

main();

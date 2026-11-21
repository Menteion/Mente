#!/usr/bin/env node
// The hook a Claude Code session of this repository runs before each shell command
// (.claude/settings.json): it reads the hook's event, JSON, from standard input, and refuses, with
// status 2 and the reason on standard error, a command that would build, test or run the
// repository's Rust code outside the local limits, naming the command to run instead
// (tools/raw-build.mjs decides; tools/local-limits.mjs). It fails closed: any fault of its own,
// an event it cannot read or an error, refuses the command and says why.
//
//   node tools/raw-build-guard.mjs < event.json

import { decide } from "./raw-build.mjs";

function refuse(words) {
  process.stderr.write(`${words}\n`);
  process.exit(2);
}

function main() {
  process.on("uncaughtException", (e) => refuse(`the raw-build guard failed, so the command did not run: ${e?.stack ?? e}`));
  let input = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (d) => (input += d));
  process.stdin.on("end", () => {
    const { status, words } = decide(input);
    if (status !== 0) refuse(words);
    process.exit(0);
  });
}

main();

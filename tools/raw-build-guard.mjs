#!/usr/bin/env node
// The hook a Claude Code session of this repository runs before each shell command
// (.claude/settings.json): it reads the hook's event, JSON, from standard input, and refuses, with
// status 2 and the reason on standard error, a command that would build, test or run the
// repository's Rust code outside the local limits, naming the command to run instead
// (tools/raw-build.mjs decides; tools/local-limits.mjs).
//
//   node tools/raw-build-guard.mjs < event.json

import { message, refusal } from "./raw-build.mjs";

function main() {
  let input = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (d) => (input += d));
  process.stdin.on("end", () => {
    let event = null;
    try {
      event = JSON.parse(input);
    } catch {
      process.exit(0);
    }
    const why = refusal(event?.tool_input?.command ?? "");
    if (!why) process.exit(0);
    process.stderr.write(`${message(why)}\n`);
    process.exit(2);
  });
}

main();

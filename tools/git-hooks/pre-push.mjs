#!/usr/bin/env node
// The pre-push hook's program (tools/git-hooks/pre-push runs it): git hands it, on standard input,
// every ref it is about to send, and it refuses, with status 1 and the reason on standard error,
// unless the tree of every commit it would send is the one tools/quick.mjs last passed in this
// checkout (tools/push.mjs, prePush). It fails closed: any fault of its own, or an error of git,
// refuses the push and says why.

import { prePush } from "../push.mjs";

function refuse(words) {
  process.stderr.write(`${words}\n`);
  process.exit(1);
}

process.on("uncaughtException", (e) => refuse(`the pre-push hook failed, so the push did not run: ${e?.stderr ?? e?.stack ?? e}`));
let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (d) => (input += d));
process.stdin.on("end", () => {
  // git gives the hook the remote's name and the URL it pushes to.
  const { status, words } = prePush(input, process.cwd(), process.argv[3]);
  if (status !== 0) refuse(words);
  process.exit(0);
});

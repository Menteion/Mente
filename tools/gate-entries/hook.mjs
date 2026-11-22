// The pre-push hook's check (briefs/gate-tools-from-main.md): <common git directory>/mente-gates/
// entry/hooks/pre-push runs it, with NODE_OPTIONS removed, before it runs anything of a copy. It
// checks the copy `current` names as every entry does, says how far it lags, and writes the copy's
// directory on standard output, for the hook to run that copy's tools/git-hooks/pre-push. It never
// reads standard input, which holds the refs git is about to send, for the copy's hook. A refusal
// ends it with 1, which refuses the push.

import path from "node:path";
import { fileURLToPath } from "node:url";
import { refusalWords, verifyCopy } from "./verify.mjs";

const GATES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checked = verifyCopy(GATES);
if (checked.refusal) {
  console.error(refusalWords(GATES, checked));
  process.exit(1);
}
for (const note of checked.notes) console.error(`mente-gates: ${note}`);
process.stdout.write(checked.copy.replace(/\\/g, "/"));

// The refresh entry (briefs/gate-tools-from-main.md): <common git directory>/mente-gates/entry/
// refresh.mjs, which tools/quick.mjs, cargo.mjs and mn.mjs run when they start, before
// requireRunner and requireHooks. It checks the copy `current` names as every entry does, except
// for the interface version: its own interface with a copy, the manifest's format and running the
// copy's tools/gates.mjs with no argument, is fixed, so that a refresh whose entries Windows left
// half replaced can still be retried. Then it runs that copy's refresher, with the gates'
// environment, and checks the copy in place afterwards. With no copy current, it runs the refresher
// of a copy an install left half done, which finishes it. It ends with 0 when a verified copy is in
// place, and with 2 and the one command that clears it when none is.

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { NODE_ENGINES, gateEnv, nodeMeetsFloor, nothingToRefresh, pendingCopy, refusalWords, verifyCopy } from "./verify.mjs";

const GATES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// The refresher runs its command only as node's entry point, which a node outside NODE_ENGINES
// cannot tell it.
if (!nodeMeetsFloor(process.versions.node)) {
  console.error(`mente-gates: the refresher needs node ${NODE_ENGINES}, not ${process.versions.node}`);
  process.exit(2);
}
const before = verifyCopy(GATES, { exempt: true });
// With no copy current, an install a kill or a refused rename left half done is finished by its
// own refresher: a copy installed but not current, or a staged one, which the check above holds.
let refresher = before.refusal ? null : before.copy;
if (before.refusal && before.missing) refresher = pendingCopy(GATES)?.dir ?? null;
if (!refresher) {
  console.error(refusalWords(GATES, before));
  process.exit(2);
}
// The fast path: the local origin/main names the verified copy and every entry holds it, so a
// refresh would ask no remote and write nothing.
if (!before.refusal && nothingToRefresh(GATES, before)) {
  for (const note of before.notes) console.error(`mente-gates: ${note}`);
  process.exit(0);
}
const r = spawnSync(process.execPath, [path.join(refresher, "tools", "gates.mjs")], { stdio: "inherit", env: gateEnv(), windowsHide: true });
if (r.error) console.error(`mente-gates: the refresher did not start (${r.error.message}), so the current copy stays`);
const after = verifyCopy(GATES, { exempt: true });
if (after.refusal) {
  console.error(refusalWords(GATES, after));
  process.exit(2);
}
for (const note of after.notes) console.error(`mente-gates: ${note}`);
process.exit(0);

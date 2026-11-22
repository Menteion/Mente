// Cargo's runner entry (briefs/gate-tools-from-main.md): the runner line of ~/.cargo/config.toml
// names <common git directory>/mente-gates/entry/capped.mjs, a path that never changes, so that
// every test binary and program cargo starts runs under the build limits of the copy `current`
// names. It checks that copy as every entry does, before importing anything of it, says how far it
// lags, and runs the copy's tools/capped.mjs, which reads its arguments as it would its own:
//
//   node <common git directory>/mente-gates/entry/capped.mjs [--mente-only] -- <command> [argument...]

import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { refusalWords, verifyCopy } from "./verify.mjs";

const GATES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checked = verifyCopy(GATES);
if (checked.refusal) {
  console.error(refusalWords(GATES, checked));
  process.exit(2);
}
for (const note of checked.notes) console.error(`mente-gates: ${note}`);
await import(pathToFileURL(path.join(checked.copy, "tools", "capped.mjs")).href);

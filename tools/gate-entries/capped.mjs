// Cargo's runner entry (briefs/gate-tools-from-main.md): the runner line of ~/.cargo/config.toml
// names <common git directory>/mente-gates/entry/capped.mjs, a path that never changes, so that
// every test binary and program cargo starts in this repository runs under the build limits of the
// copy `current` names. The runner line serves every Rust project of the machine: with
// --mente-only, a command whose directory lies outside a checkout of this repository runs
// untouched, decided here, before any copy is checked or read. Otherwise it checks that copy as
// every entry does, says how far it lags, and runs the copy's tools/capped.mjs, which reads its
// arguments as it would its own:
//
//   node <common git directory>/mente-gates/entry/capped.mjs [--mente-only] -- <command> [argument...]

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { inMente, refusalWords, verifyCopy } from "./verify.mjs";

const GATES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const argv = process.argv.slice(2);
if (argv[0] === "--mente-only" && argv[1] === "--" && argv.length > 2 && !inMente(process.cwd())) {
  const r = spawnSync(argv[2], argv.slice(3), { stdio: "inherit", env: process.env });
  if (r.error) console.error(`mente-gates: ${argv[2]} did not start: ${r.error.message}`);
  process.exit(r.status ?? 1);
}
const checked = verifyCopy(GATES);
if (checked.refusal) {
  console.error(refusalWords(GATES, checked));
  process.exit(2);
}
for (const note of checked.notes) console.error(`mente-gates: ${note}`);
await import(pathToFileURL(path.join(checked.copy, "tools", "capped.mjs")).href);

#!/usr/bin/env node
// One command under the build limits (tools/local-limits.mjs), its arguments passed as given, with
// no slot: for cargo's runner, which hands each test binary and each program `cargo run` starts to
// it, inside a cargo that tools/cargo.mjs or tools/quick.mjs already runs in a slot, or that ran
// raw. With --mente-only, a command whose directory lies outside a checkout of this repository
// runs untouched: the runner set in the user's ~/.cargo/config.toml serves every project of the
// machine.
//
//   node tools/capped.mjs [--mente-only] -- <command> [argument...]

import { BUILD, runCapped } from "./local-limits.mjs";

const argv = process.argv.slice(2);
const menteOnly = argv[0] === "--mente-only";
const rest = menteOnly ? argv.slice(1) : argv;
if (rest[0] !== "--" || rest.length < 2) {
  console.error("usage: node tools/capped.mjs [--mente-only] -- <command> [argument...]");
  process.exit(2);
}
process.exit(runCapped(rest.slice(1), BUILD, { menteOnly }));

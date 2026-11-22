#!/usr/bin/env node
// Any local cargo command, in one of the machine's build slots and under the build limits
// (tools/local-limits.mjs): each process at most 8 GB committed, all of them at most 16 GB, the
// whole run at most 30 minutes unless --minutes says otherwise, at half the cores.
//
//   node tools/cargo.mjs [--minutes N] <cargo arguments>
//   node tools/cargo.mjs test -p mente-sema frames
//   node tools/cargo.mjs --minutes 60 build --release

import { BUILD, acquireSlot, minutesFrom, requireRunner, runCapped } from "./local-limits.mjs";

const { args, limits } = minutesFrom(process.argv.slice(2), BUILD);
if (args.length === 0) {
  console.error("usage: node tools/cargo.mjs [--minutes N] <cargo arguments>");
  process.exit(2);
}
if (!requireRunner()) process.exit(2);
const slot = acquireSlot(`cargo ${args.join(" ")}`);
const status = runCapped(["cargo", ...args], limits);
slot.release();
process.exit(status);

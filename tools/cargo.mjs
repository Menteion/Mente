#!/usr/bin/env node
// Any local cargo command, in one of the machine's build slots and under the build limits
// (tools/local-limits.mjs): each process at most 8 GB committed, all of them at most 16 GB, the
// whole run at most 30 minutes unless --minutes says otherwise, at half the cores.
//
//   node tools/cargo.mjs [--minutes N] <cargo arguments>
//   node tools/cargo.mjs test -p mente-sema frames
//   node tools/cargo.mjs --minutes 60 build --release

import path from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD, acquireSlot, minutesFrom, refreshGates, requireHooks, requireRunner, runCapped } from "./local-limits.mjs";

// The checkout these tools belong to, whose gates they refresh and whose hooks they require,
// wherever they are run from.
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const { args, limits } = minutesFrom(process.argv.slice(2), BUILD);
if (args.length === 0) {
  console.error("usage: node tools/cargo.mjs [--minutes N] <cargo arguments>");
  process.exit(2);
}
const gates = refreshGates({ cwd: ROOT });
if (!gates) process.exit(2);
// FALLBACK: with no copy installed, today's working-tree gates (local-limits.mjs, refreshGates).
const fallback = gates === "fallback";
if (!requireRunner({ fallback })) process.exit(2);
if (!requireHooks({ cwd: ROOT, fallback })) process.exit(2);
const slot = acquireSlot(`cargo ${args.join(" ")}`);
const status = runCapped(["cargo", ...args], limits);
slot.release();
process.exit(status);

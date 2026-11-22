#!/usr/bin/env node
// An ad-hoc run of mn, for a probe or a review, in one of the machine's build slots, held from the
// build to the end of the run (the review of 8f03074: two builds and probes beside them could pass
// the machine's commit limit): it builds this checkout's mn under the build limits, then runs it
// under the probe limits (tools/local-limits.mjs): each process at most 4 GB committed, all of
// them at most 8 GB, at most 5 minutes unless --minutes says otherwise. Paths in the arguments are
// read from the current directory.
//
//   node tools/mn.mjs [--minutes N] <mn arguments>
//   node tools/mn.mjs check examples/01_holographic_module.mn

import path from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD, PROBE, acquireSlot, minutesFrom, refreshGates, requireHooks, requireRunner, runCapped } from "./local-limits.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const { args, limits } = minutesFrom(process.argv.slice(2), PROBE);
if (!refreshGates({ cwd: ROOT })) process.exit(2);
if (!requireRunner()) process.exit(2);
if (!requireHooks({ cwd: ROOT })) process.exit(2);
const slot = acquireSlot(`mn ${args.join(" ")} (tools/mn.mjs)`);
let status = runCapped(["cargo", "build", "--quiet", "--bin", "mn"], BUILD, { cwd: ROOT });
if (status === 0) {
  const target = process.env.CARGO_TARGET_DIR ? path.resolve(ROOT, process.env.CARGO_TARGET_DIR) : path.join(ROOT, "target");
  const mn = path.join(target, "debug", process.platform === "win32" ? "mn.exe" : "mn");
  status = runCapped([mn, ...args], limits);
}
slot.release();
process.exit(status);
